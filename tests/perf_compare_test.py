from pathlib import Path
import sys
import json
import tempfile
import subprocess
from perf_test import process, config, event
import unittest
sys.path.insert(0,str(Path(__file__).parents[1]/'examples'))
import perf_compare as compare


def raw_campaign(root, rss=4096, missing_cleanup=False, seconds=5):
    """Typed records of a capped source with measured residency and cleanup."""
    root.mkdir(parents=True)
    (root/'campaign.json').write_text(json.dumps(dict(status='finished',samples_executed=9,
        aggregate_censored=False,limits=dict(per_sample_seconds=seconds,memory_mib=1024))))
    for i in range(9):
        cfg=config();cfg['data'].update(max_ticks=50000000,timeout_seconds=3)
        result=event('INCOMPLETE');result['data'].update(source_goal_reached=False,
            censor_reason='source_timeout',native_peak_rss_estimate_kib=rss,
            cleanup_done=not (missing_cleanup and i==8))
        result['data']['times_ms'].update(source_delivery=3000.,cleanup=12.)
        result['data']['work'].update(answers=0,applications=100,advance1_ticks=5000)
        proc=process(2,'censored');proc.update(wall_seconds=3.02,max_rss_kib=rss,
            limits=dict(wall_seconds=seconds,address_space_mib_per_process=1024))
        (root/f'{i}.json').write_text(json.dumps(dict(status='censored',warmup=False,
            process=proc,records=[cfg,result])))


class ComparisonTests(unittest.TestCase):
    def test_censored_raw_resources_and_missing_cleanup_keep_all_observations(self):
        with tempfile.TemporaryDirectory() as directory:
            before=Path(directory)/'before';after=Path(directory)/'after'
            raw_campaign(before);raw_campaign(after,rss=65536,missing_cleanup=True)
            _,_,samples,values=compare.load(after)
            self.assertEqual(len(values),len(samples))
            self.assertEqual([v.get('workload.native_peak_rss_estimate_kib') for v in values],[65536]*9)
            self.assertIsNone(values[-1].get('workload.times_ms.cleanup'))
            self.assertTrue(all(v.get('workload.times_ms.source_delivery') is None for v in values))
            report=compare.comparison(before,after,['workload.native_peak_rss_estimate_kib',
                'workload.times_ms.cleanup','workload.times_ms.source_delivery'])
            self.assertEqual(report['status'],'censored_comparison')
            self.assertEqual(report['metrics']['workload.native_peak_rss_estimate_kib']['status'],'increase')
            for key in ('workload.times_ms.cleanup','workload.times_ms.source_delivery'):
                self.assertEqual(report['metrics'][key]['status'],'unavailable')
            self.assertEqual(report['sample_paths']['after'][-1],str(after/'8.json'))

    def test_censored_comparisons_require_matching_external_and_native_budgets(self):
        with tempfile.TemporaryDirectory() as directory:
            before=Path(directory)/'before';after=Path(directory)/'after'
            raw_campaign(before);raw_campaign(after,seconds=10)
            with self.assertRaisesRegex(ValueError,'limits'):
                compare.comparison(before,after,['workload.native_peak_rss_estimate_kib'])
            metadata=json.loads((after/'campaign.json').read_text());metadata['limits']['per_sample_seconds']=5
            (after/'campaign.json').write_text(json.dumps(metadata))
            with self.assertRaisesRegex(ValueError,'limits'):
                compare.comparison(before,after,['workload.native_peak_rss_estimate_kib'])
            for i in range(9):
                path=after/f'{i}.json';sample=json.loads(path.read_text())
                sample['process']['limits']['wall_seconds']=5
                sample['records'][0]['data']['max_ticks']=100
                path.write_text(json.dumps(sample))
            with self.assertRaisesRegex(ValueError,'configuration'):
                compare.comparison(before,after,['workload.native_peak_rss_estimate_kib'])

    def test_missing_interval_is_not_a_selected_subset(self):
        report=compare.assess([{'cost':1}]*9,[{'cost':10}]*8+[{}],['cost'])
        self.assertEqual(report['cost']['status'],'unavailable')

    def test_exact_complete_separation_and_ties(self):
        p,method,n=compare.permutation_probability([1,2,3],[4,5,6])
        self.assertEqual((p,method,n),(.1,'exact',20))
        self.assertEqual(compare.permutation_probability([1]*5,[1]*5)[0],1)
        self.assertEqual(compare.permutation_probability([1,2,3],[4,5,6]),compare.permutation_probability([4,5,6],[1,2,3]))

    def test_small_samples_and_zero_denominators_remain_undefined(self):
        self.assertEqual(compare.compare_metric([1],[10])['status'],'insufficient_evidence')
        self.assertIsNone(compare.compare_metric([0]*5,[1]*5)['ratio'])
        self.assertEqual(compare.compare_metric([],[1]*5)['status'],'unavailable')

    def test_large_shift_detected_unchanged_control_uncertain(self):
        before=[{'cost':v,'control':v} for v in range(1,8)]
        after=[{'cost':v+100,'control':v} for v in range(1,8)]
        result=compare.assess(before,after,['cost','control','missing'])
        self.assertEqual(result['cost']['status'],'increase')
        self.assertEqual(result['control']['status'],'uncertain')
        self.assertEqual(result['missing']['status'],'unavailable')

    def test_same_mean_different_distributions_is_not_a_mean_change(self):
        self.assertEqual(compare.permutation_probability([0,0,5,5,5],[3,3,3,3,3])[0],1)

    def test_raw_outcomes_validation_and_censoring(self):
        with tempfile.TemporaryDirectory() as directory:
            root=Path(directory)
            sample=dict(warmup=False,status='completed',process=process(),records=[config(),event()])
            metadata=dict(samples_executed=1,aggregate_censored=False,summary={'counts':{'invented':99}})
            (root/'campaign.json').write_text(json.dumps(metadata))
            (root/'0.json').write_text(json.dumps(sample))
            self.assertEqual(compare.comparison(root,root,['workload.times_ms.source_delivery'])['before_outcomes'],{'completed':1})
            sample['records'].append(event())
            (root/'0.json').write_text(json.dumps(sample))
            with self.assertRaises(ValueError): compare.load(root)
            for status,expected in [('censored','censored_comparison'),('supervisor_error','failed_comparison'),('cleanup_failure','failed_comparison')]:
                (root/'0.json').write_text(json.dumps(dict(warmup=False,status=status,records=[])))
                self.assertEqual(compare.comparison(root,root,['cost'])['status'],expected)
            for invalid in ('NaN', '{"process":{"user_cpu_seconds":1e400}}'):
                (root/'0.json').write_text(invalid)
                with self.assertRaises(ValueError): compare.load(root)

    def test_observer_comparison_allows_only_explicit_measurement_mode_changes(self):
        with tempfile.TemporaryDirectory() as directory:
            roots=[Path(directory)/name for name in ('baseline','observed')]
            for index,root in enumerate(roots):
                root.mkdir();cfg=config();cfg['data']['diagnostics_feature']=bool(index)
                (root/'campaign.json').write_text(json.dumps(dict(samples_executed=1,aggregate_censored=False)))
                (root/'0.json').write_text(json.dumps(dict(warmup=False,status='completed',process=process(),records=[cfg,event()])))
            with self.assertRaises(ValueError):compare.comparison(*roots,['workload.times_ms.source_delivery'])
            report=compare.comparison(*roots,['workload.times_ms.source_delivery'],True)
            self.assertEqual(report['observation_changes'],{'diagnostics_feature':{'before':False,'after':True}})
            sample=json.loads((roots[1]/'0.json').read_text())
            sample['records'][0]['data']['size']=9;sample['records'][1]['data']['size']=9
            (roots[1]/'0.json').write_text(json.dumps(sample))
            with self.assertRaises(ValueError):compare.comparison(*roots,['workload.times_ms.source_delivery'],True)

    def test_processing_deadline_is_an_explicit_outcome(self):
        with tempfile.TemporaryDirectory() as directory:
            root=Path(directory); out=root/'result.json'
            # A bounded processing stand-in keeps the alarm inside actual work,
            # independent of filesystem speed and scheduler microsecond races.
            script="import sys,time;sys.path.insert(0,sys.argv.pop(1));import perf_compare as c;c.comparison=lambda *a: time.sleep(2);raise SystemExit(c.main())"
            result=subprocess.run([sys.executable,'-c',script,str(Path(compare.__file__).parent),str(root),str(root),'--out',str(out),'--seconds','0.05'],capture_output=True,text=True,timeout=3)
            self.assertEqual(result.returncode,2,result.stderr)
            self.assertEqual(json.loads(out.read_text())['status'],'comparison_timeout')

    def test_seeded_monte_carlo_is_reproducible(self):
        a=list(range(20));b=list(range(20,40))
        self.assertEqual(compare.permutation_probability(a,b),compare.permutation_probability(a,b))


if __name__=='__main__': unittest.main()
