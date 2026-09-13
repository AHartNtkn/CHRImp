from pathlib import Path
import sys
import unittest
import json
import tempfile
from perf_test import process, config, event
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

    def test_deep_family_has_enough_resolution_for_a_large_regression(self):
        control=[10+i*.01 for i in range(9)]
        control=[10+i*.01 for i in range(12)]
        draws=regress.permutation_draws(100,5)
        signal=regress.evaluate(control,[v*3 for v in control],control,control,draws=draws)
        findings=[signal]+[dict(status='uncertain',p_value=1.,noise_span=1.,absolute_change=0.,method='monte_carlo',assignments=draws) for _ in range(999)]
        regress.correct(findings)
        self.assertEqual(signal['status'],'regression')
        weak=regress.evaluate(control,[v*3 for v in control],control,control)
        regress.correct([weak]+findings[1:])
        self.assertEqual(weak['status'],'resolution_limited')

    def test_exact_two_sided_resolution_requires_both_complements(self):
        a=[1,2,3,4,5];b=[101,102,103,104,105]
        finding=regress.evaluate(a,b,a,a)
        controls=[dict(status='uncertain',p_value=1.,noise_span=1.,absolute_change=0.,method='exact',assignments=252,before_n=5,after_n=5) for _ in range(9)]
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

    def test_noise_controls_prevent_small_qualified_change_becoming_regression(self):
        base=[10.]*9;after=[10.1]*9
        finding=regress.evaluate(base,after,[9.8]*9,[10.2]*9)
        regress.correct([finding])
        self.assertEqual(finding['status'],'within_control_spread')
        self.assertEqual(regress.evaluate(base,after,[],[])['status'],'uncalibrated')

    def test_ratio_zero_and_incomplete_values_are_undefined(self):
        def p(a,status='completed'): return dict(status=status,values=[{'x':v} for v in a])
        self.assertEqual(regress.ratio_values(p([2,4]),p([6,8]),'x'),[3,2])
        self.assertEqual(regress.ratio_values(p([0,4]),p([6,8]),'x'),[])
        self.assertEqual(regress.ratio_values(p([2,4]),p([6,8],'censored'),'x'),[])
        self.assertEqual(regress.values(p([2,None]),'x'),[])

    def test_scaling_worsening_can_be_detected_without_absolute_slowdown(self):
        def p(a): return dict(status='completed',values=[{'x':v} for v in a])
        low=p([10+i*.01 for i in range(9)]);high=p([20+i*.01 for i in range(9)])
        baseline=regress.ratio_values(low,high,'x')
        after=regress.ratio_values(p([1+i*.001 for i in range(9)]),p([10+i*.01 for i in range(9)]),'x')
        finding=regress.evaluate(baseline,after,baseline,baseline)
        regress.correct([finding])
        self.assertEqual(finding['status'],'regression')


if __name__=='__main__': unittest.main()
