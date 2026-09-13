from pathlib import Path
import sys
import unittest
import json
import tempfile
import math
from unittest.mock import patch
from perf_test import process, config, event
from perf_compare_test import raw_campaign
sys.path.insert(0,str(Path(__file__).parents[1]/'examples'))
import perf_regress as regress


class RegressionTests(unittest.TestCase):
    def test_large_cost_increase_detected_across_multiple_metrics(self):
        controls=[10+i*.01 for i in range(9)]
        findings=[regress.evaluate(controls,[x*2 for x in controls],controls,controls)]
        findings += [regress.evaluate(controls,controls,controls,controls) for _ in range(49)]
        regress.correct(findings)
        self.assertEqual(findings[0]['status'],'regression')
        self.assertTrue(all(f['status']=='uncertain' for f in findings[1:]))
        self.assertAlmostEqual(findings[0]['noise_span'],.08)

    def test_large_family_reports_resolution_limit_without_growing_resampling(self):
        control=[10+i*.01 for i in range(12)]
        signal=regress.evaluate(control,[v*3 for v in control],control,control)
        findings=[signal]+[dict(status='uncertain',p_value=1.,noise_resolution=1.,
            absolute_change=0.,method='monte_carlo',assignments=9999) for _ in range(999)]
        regress.correct(findings)
        self.assertEqual(signal['assignments'],9999)
        self.assertEqual(signal['status'],'resolution_limited')

    def test_censored_raw_rss_growth_and_missing_cleanup_use_the_default_path(self):
        with tempfile.TemporaryDirectory() as tmp:
            roots=[]
            for name,rss in [('ca',4096),('cb',4096),('before',4096),('after',65536)]:
                root=Path(tmp)/name;roots.append(root)
                raw_campaign(root/'rewrite-8',rss=rss,missing_cleanup=name=='after')
                (root/'suite.json').write_text(json.dumps(dict(status='censored',points=[dict(
                    id='rewrite-8',status='censored',family='rewrite',axis='size',value=8,
                    workload=['rewrite','8'])])))
            for metrics in (None,['workload.native_peak_rss_estimate_kib','workload.times_ms.cleanup',
                                  'workload.times_ms.source_delivery']):
                report=regress.compare(*roots,metrics)
                self.assertEqual(report['status'],'regression')
                self.assertTrue(report['incomplete_evidence'])
                self.assertEqual(report['permutation_draws'],9999)
                findings={f['metric']:f for f in report['findings']}
                self.assertEqual(findings['workload.native_peak_rss_estimate_kib']['status'],'regression')
                for key in ('workload.times_ms.cleanup','workload.times_ms.source_delivery'):
                    self.assertEqual(findings[key]['status'],'unavailable')
                self.assertEqual(report['sample_paths']['rewrite-8'][3][-1],str(roots[3]/'rewrite-8/8.json'))
            control=regress.compare(roots[0],roots[1],roots[2],roots[2])
            self.assertEqual(control['status'],'incomplete_evidence')
            self.assertEqual(control['regression_count'],0)
            metadata=json.loads((roots[3]/'rewrite-8/campaign.json').read_text())
            metadata['limits']['per_sample_seconds']=10
            (roots[3]/'rewrite-8/campaign.json').write_text(json.dumps(metadata))
            with self.assertRaisesRegex(ValueError,'limits'):
                regress.compare(*roots)

    def test_completed_campaign_accepts_varied_external_limits(self):
        with tempfile.TemporaryDirectory() as tmp:
            root=Path(tmp);campaign=root/'rewrite-8';raw_campaign(campaign)
            for i in range(9):
                path=campaign/f'{i}.json';sample=json.loads(path.read_text())
                sample['status']='completed';sample['process'].update(status='completed',returncode=0)
                sample['process']['limits']['wall_seconds']=5+i
                sample['records'][1]=event()
                path.write_text(json.dumps(sample))
            (root/'suite.json').write_text(json.dumps(dict(status='completed',points=[dict(
                id='rewrite-8',status='completed',family='rewrite',axis='size',value=8,
                workload=['rewrite','8'])])))
            _,points=regress.read_suite(root)
            self.assertEqual(points['rewrite-8']['status'],'completed')
            self.assertEqual(len(points['rewrite-8']['values']),9)

    def test_exact_two_sided_resolution_requires_both_complements(self):
        a=[1,2,3,4,5];b=[101,102,103,104,105]
        finding=regress.evaluate(a,b,a,a)
        controls=[dict(status='uncertain',p_value=1.,noise_resolution=1.,absolute_change=0.,method='exact',assignments=252,before_n=5,after_n=5) for _ in range(9)]
        regress.correct([finding]+controls)
        self.assertEqual(finding['status'],'resolution_limited')
        self.assertEqual(finding['permutation_resolution_lower_bound'],2/252)

    def test_report_failure_survives_successful_sample_campaign(self):
        with tempfile.TemporaryDirectory() as tmp:
            root=Path(tmp);point=root/'rewrite-8';point.mkdir()
            (root/'suite.json').write_text(json.dumps(dict(status='failed',points=[dict(id='rewrite-8',status='failed',family='rewrite',axis='size',value=8,workload=['rewrite','8'])])))
            (point/'campaign.json').write_text(json.dumps(dict(status='finished',samples_executed=1,aggregate_censored=False)))
            (point/'0.json').write_text(json.dumps(dict(status='completed',warmup=False,process=process(),records=[config(),event()])))
            _,points=regress.read_suite(root)
            self.assertEqual(points['rewrite-8']['status'],'failed')
            self.assertEqual(points['rewrite-8']['campaign_status'],'completed')
            report=regress.compare(root,root,root,root,['workload.times_ms.source_delivery'])
            self.assertEqual(report['status'],'failed_evidence')
            self.assertEqual(report['outcome_layers']['rewrite-8'][0],dict(suite='failed',campaign='completed'))

    def test_allocation_growth_uses_raw_campaign_records(self):
        with tempfile.TemporaryDirectory() as tmp:
            suites=[]
            for name,multiplier in [('ca',1),('cb',1),('before',1),('after',4)]:
                root=Path(tmp)/name;point=root/'rewrite-8';point.mkdir(parents=True);suites.append(root)
                (root/'suite.json').write_text(json.dumps(dict(status='completed',points=[dict(id='rewrite-8',status='completed',family='rewrite',axis='size',value=8,workload=['rewrite','8'])])))
                (point/'campaign.json').write_text(json.dumps(dict(status='finished',samples_executed=9,aggregate_censored=False)))
                for i in range(9):
                    cfg=config();cfg['data']['diagnostics_feature']=True
                    allocation=dict(schema=1,kind='allocations',data=dict(process_live_requested_bytes=0,
                        process_peak_requested_bytes=1048576*multiplier,phases=[dict(phase='engine',
                        allocations=1,deallocations=1,allocated_bytes=1048576*multiplier,freed_bytes=1048576*multiplier)]))
                    (point/f'{i}.json').write_text(json.dumps(dict(status='completed',warmup=False,process=process(),records=[cfg,event(),allocation])))
            report=regress.compare(*suites,['allocations.total_allocated_bytes','allocations.process_peak_requested_bytes'])
            self.assertEqual(report['status'],'regression')
            self.assertEqual(report['regression_count'],2)
            defaults=regress.compare(*suites)
            allocation_findings=[f for f in defaults['findings'] if f['metric'].startswith('allocations.')]
            self.assertEqual(len(allocation_findings),2)
            self.assertTrue(all(f['status']=='regression' for f in allocation_findings))

    def test_noise_controls_prevent_small_qualified_change_becoming_regression(self):
        base=[10.]*9;after=[10.1]*9
        finding=regress.evaluate(base,after,[9.8]*9,[10.2]*9)
        regress.correct([finding])
        self.assertEqual(finding['status'],'within_control_spread')
        self.assertEqual(regress.evaluate(base,after,[],[])['status'],'unavailable')

    def test_independent_scaling_is_invariant_to_all_input_orders(self):
        low=[2,3,5,7,11,13,17]; high=[3,5,7,11,13,17,19,23,29]
        baseline=(low,high)
        after=([v*.5 for v in low],[v*2 for v in high])
        first=regress.scaling(baseline,after,baseline,baseline,draws=999)
        reverse=lambda pair: tuple(list(reversed(a)) for a in pair)
        second=regress.scaling(reverse(baseline),reverse(after),reverse(baseline),reverse(baseline),draws=999)
        self.assertEqual(first,second)
        self.assertEqual(first['before_n'],[7,9])
        self.assertAlmostEqual(first['before_ratio'],sum(high)/len(high)/(sum(low)/len(low)))
        self.assertEqual(first['operands']['before'],dict(low=low,high=high))

    def test_ratio_undefined_does_not_drop_bootstrap_samples(self):
        normal=([2]*9,[4]*9)
        for invalid in (([],[4]*9),([0]*9,[4]*9),([0]*8+[1],[4]*9)):
            f=regress.scaling(invalid,normal,normal,normal,draws=999)
            self.assertEqual(f['status'],'unavailable')
            self.assertNotIn('p_value',f)
        sparse=regress.scaling(([2]*4,[4]*9),normal,normal,normal,draws=999)
        self.assertEqual(sparse['status'],'insufficient_evidence')
        zero_high=regress.scaling(normal,([2]*9,[0]*9),normal,normal,draws=999)
        regress.correct([zero_high])
        self.assertEqual(zero_high['status'],'improvement')
        def p(a,status='completed'): return dict(status=status,values=[{'x':v} for v in a])
        self.assertEqual(regress.values(p([2,None]),'x'),[])
        self.assertEqual(regress.values(p([2,4],'censored'),'x'),[2,4])

    def test_proportional_cost_change_preserves_scaling(self):
        baseline=([19,20,20,21,22,23]*2,[80,80,83,87,90]*3)
        after=tuple([v*.3 for v in a] for a in baseline)
        f=regress.scaling(baseline,after,baseline,baseline,draws=999)
        regress.correct([f])
        self.assertEqual(f['status'],'uncertain')
        self.assertAlmostEqual(f['absolute_change'],0.)

    def test_scaled_decimal_constants_are_numerically_equivalent(self):
        baseline=([1.]*12,[3.]*12)
        f=regress.scaling(baseline,([.1]*12,[.3]*12),baseline,baseline,draws=999)
        regress.correct([f])
        self.assertEqual(f['status'],'uncertain')

    def test_small_distribution_change_must_qualify_shared_family(self):
        a=[10]*12;b=[4]*6+[16]*6
        f=regress.evaluate(a,b,a,a)
        regress.correct([f])
        self.assertEqual(f['status'],'uncertain')
        self.assertGreater(f['distribution']['p_value'],f['distribution']['holm_threshold'])
        self.assertEqual(f['distribution']['after_above_before_max'],6)

    def test_scaling_worsening_despite_both_sizes_getting_faster(self):
        baseline=([19,20,20,21,22,23]*2,[80,80,83,87,90]*3)
        after=([v*.25 for v in baseline[0]],[v*.75 for v in baseline[1]])
        f=regress.scaling(baseline,after,baseline,baseline,draws=1999)
        regress.correct([f])
        self.assertEqual(f['status'],'regression')
        self.assertAlmostEqual(f['after_ratio']/f['before_ratio'],3.)
        self.assertGreater(f['approximate_95_percent_interval'][0],0)

    def test_control_outlier_is_retained_without_vetoing_shift(self):
        for outlier in (150,10000):
            control=[99,100,100,100,100,100,100,101,outlier]
            f=regress.evaluate([100]*9,[125]*9,control,[100]*9)
            regress.correct([f])
            self.assertEqual(f['status'],'regression')
            self.assertEqual(f['control_range'],[99,outlier])
            self.assertEqual(f['noise_span'],outlier-99)
            self.assertEqual(f['noise_resolution'],0)

    def test_distribution_change_has_no_automatic_adverse_direction(self):
        # Mean 10 on both sides; compare concentration to bimodality, both ways.
        a=[10]*24;b=[4]*12+[16]*12
        for before,after in ((a,b),(b,a)):
            f=regress.evaluate(before,after,before,before)
            regress.correct([f])
            self.assertEqual(f['mean_status'],'uncertain')
            self.assertEqual(f['status'],'distribution_change')
            self.assertEqual(f['distribution']['status'],'distribution_change')
            self.assertEqual(f['absolute_change'],0)

    def test_single_extreme_is_visible_without_rare_event_certainty(self):
        a=[10]*9;b=[10]*8+[10000]
        f=regress.evaluate(a,b,a,a)
        regress.correct([f])
        self.assertEqual(f['status'],'uncertain')
        self.assertEqual(f['distribution']['status'],'uncertain')
        self.assertEqual(f['distribution']['after_above_before_max'],1)
        self.assertEqual(f['distribution']['after_range'],[10,10000])

    def test_distribution_permutation_ties_and_exact_small_case(self):
        a=[0]*5;b=[1]*5
        f=regress.distribution(a,b,a,a,999)
        self.assertEqual(f['p_value'],2/math.comb(10,5))
        self.assertEqual(f['statistic'],1)
        tied=regress.distribution(a,a,a,a,999)
        self.assertEqual(tied['p_value'],1)
        self.assertEqual(tied['statistic'],0)

    def test_mean_and_distribution_use_one_holm_family(self):
        f=dict(status='uncertain',p_value=.02,noise_resolution=0,absolute_change=1,
               method='monte_carlo',assignments=9999,
               distribution=dict(status='uncertain',p_value=.04,noise_resolution=0,
                                 statistic=1,method='monte_carlo',assignments=9999))
        null=dict(status='uncertain',p_value=1,noise_resolution=0,absolute_change=0,
                  method='independent_bootstrap',assignments=9999)
        regress.correct([f,null])
        self.assertEqual(f['status'],'uncertain')
        self.assertAlmostEqual(f['holm_threshold'],.05/3)
        self.assertEqual(f['distribution']['status'],'uncertain')

    @staticmethod
    def suite(low,high,status='completed'):
        points={}
        for size,rows in ((1,low),(2,high)):
            key='x-'+str(size)
            spec=dict(id=key,status=status,family='x',axis='size',value=size,workload=['rewrite',str(size)])
            points[key]=dict(spec=spec,configuration={'case':'rewrite','size':size},values=rows,
                             status=status,suite_status=status,campaign_status=status)
        return dict(status=status),points

    def test_full_compare_order_and_coupled_resource_cost_changes(self):
        metric='workload.native_peak_rss_estimate_kib'
        x=[10]*12+[30]*12
        rows=lambda a:[{'cost':v,metric:v*100} for v in a]
        before=self.suite(rows(x),rows([2*v for v in x]))
        reordered=self.suite(rows(x[::-1]),rows([2*v for v in x[::-1]]))
        with patch.object(regress,'read_suite',side_effect=[before,before,before,reordered]):
            report=regress.compare('a','b','old','new',['cost',metric])
        self.assertEqual(report['status'],'no_regression_detected')
        for f in report['findings']:
            self.assertEqual(f['status'],'uncertain')
        # Different fresh case: costs halve while residency triples.
        stable=self.suite([{'cost':10,metric:100}]*12,[{'cost':20,metric:200}]*12)
        after=self.suite([{'cost':5,metric:300}]*12,[{'cost':10,metric:600}]*12)
        with patch.object(regress,'read_suite',side_effect=[stable,stable,stable,after]):
            report=regress.compare('a','b','old','new',['cost',metric])
        self.assertEqual(report['status'],'regression')
        self.assertEqual(report['regression_count'],2)
        self.assertTrue(all(f['status']=='uncertain' for f in report['findings'] if f['kind']=='scaling'))

    def test_completed_mean_preserving_changes_and_censoring_have_distinct_outcomes(self):
        rows=lambda a:[{'cost':v} for v in a]
        before=self.suite(rows([10]*24),rows([20]*24))
        changed=self.suite(rows([4]*12+[16]*12),rows([8]*12+[32]*12))
        with patch.object(regress,'read_suite',side_effect=[before,before,before,changed]):
            report=regress.compare('a','b','old','new',['cost'])
        self.assertEqual(report['status'],'distribution_change')
        self.assertEqual(report['regression_count'],0)
        censored=self.suite(rows([None]*12),rows([None]*12),'censored')
        with patch.object(regress,'read_suite',side_effect=[before,before,before,censored]):
            report=regress.compare('a','b','old','new',['cost'])
        self.assertEqual(report['status'],'incomplete_evidence')
        self.assertEqual(report['regression_count'],0)



if __name__=='__main__': unittest.main()
