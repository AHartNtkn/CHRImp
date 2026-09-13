import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch
import io
import contextlib
sys.path.insert(0,str(Path(__file__).parents[1]/'examples'))
import perf_suite as suite


class SuiteTests(unittest.TestCase):
    def test_plan_has_distinct_axes_and_reproducible_graph_seeds(self):
        routine=suite.plan('routine',0);deep=suite.plan('deep',7)
        self.assertEqual(deep,suite.plan('deep',7))
        self.assertEqual(len({p['id'] for p in deep}),len(deep))
        self.assertTrue({p['family'] for p in routine}<={p['family'] for p in deep})
        self.assertTrue({'size','heads','uses','cadence','work'}<={p['axis'] for p in deep})
        for case in ('life-held-output-conditional','life-archive-fixed-conditional','life-archive-rotate-conditional','life-inspections-conditional'):
            self.assertEqual({p['axis'] for p in deep if p['workload'][0]==case},{'size','work'})
        self.assertTrue(all(p['risk'] for p in deep))

    def test_saved_synthesis_queries_have_unrestricted_deep_cases(self):
        inventory=suite.notebook_inventory()
        self.assertEqual(len(inventory),85)
        generation=[q for q in inventory if q['kind']=='unrestricted_synthesis']
        self.assertEqual(len(generation),17)
        planned={p['workload'][0] for p in suite.plan('deep',0)}
        self.assertTrue(all(q['case'] in planned for q in generation))
        self.assertTrue(all(q['name'].startswith('Synthesize ') for q in generation))
        self.assertTrue(all(q['case'] is None for q in inventory if q['kind']!='unrestricted_synthesis'))
        self.assertEqual(len({(q['notebook'],q['id']) for q in inventory}),85)

    def test_scaling_preserves_operands_and_censored_gaps(self):
        def point(value,cost,status='completed'):
            return dict(family='x',axis='size',value=value,status=status,metrics={'workload.times_ms.source_delivery':{'median':cost}})
        r=suite.scaling([point(2,4),point(4,16),point(8,64,'censored'),point(16,256)])
        self.assertEqual(r[0]['metrics']['workload.times_ms.source_delivery'],dict(small=4,large=16,ratio=4,exponent=2))
        self.assertEqual([p['status'] for p in r],['observed','incomplete_scaling','incomplete_scaling'])
        r=suite.scaling([point(2,0),point(4,1)])[0]['metrics']['workload.times_ms.source_delivery']
        self.assertIsNone(r['ratio']);self.assertIsNone(r['exponent'])

    def test_round_robin_coverage_partial_reports_and_failures(self):
        for stop, failure in [(None,False),(3,False),(4,True)]:
            with self.subTest(stop=stop,failure=failure), tempfile.TemporaryDirectory() as tmp:
                clock=[0.];events=[];closed=[];calls=[]
                def campaign(argv, absolute_deadline=None, progress=None):
                    directory=Path(argv[argv.index('--out')+1]);directory.mkdir()
                    name=directory.name;counts={};calls.append(argv)
                    try:
                        for i in range(3):
                            status='process_failure' if failure and not events else 'completed'
                            counts[status]=counts.get(status,0)+1
                            events.append((name,i))
                            if stop is not None and len(events)==stop: clock[0]=20.
                            yield
                        return 1 if 'process_failure' in counts else 0
                    finally:
                        closed.append(name)
                        (directory/'campaign.json').write_text(json.dumps(dict(
                            status='finished',aggregate_censored=sum(counts.values())<3,
                            summary=dict(counts=counts,metrics={'workload.work.applications':{'median':4}}))))
                argv=['perf_suite.py','routine','--binary','/bin/true','--only','rewrite',
                      '--repeat','3','--seconds','10','--out',str(Path(tmp)/'suite')]
                with patch.object(sys,'argv',argv),patch.object(suite.perf,'campaign',campaign,create=True),patch.object(suite.time,'monotonic',lambda:clock[0]),contextlib.redirect_stdout(io.StringIO()):
                    code=suite.main()
                report=json.loads((Path(tmp)/'suite/suite.json').read_text())
                order=[p['id'] for p in report['points']]
                self.assertEqual(events,[(name,i) for i in range(3) for name in order][:stop])
                self.assertCountEqual(closed,order)
                self.assertTrue(all(a[a.index('--warmup')+1]=='0' for a in calls))
                self.assertEqual(code,1 if failure else (2 if stop else 0))
                self.assertEqual(sum(sum(p['outcomes'].values()) for p in report['points']),len(events))
                if stop:
                    self.assertTrue(all(p['status']!='completed' for p in report['points']))
                else:
                    self.assertTrue(all(p['status']=='completed' for p in report['points']))

    def test_deadline_and_malformed_report_finalize_all_admitted(self):
        for behavior,expected in [('deadline','censored'),('failed_deadline','failed'),('malformed','failed')]:
            with self.subTest(behavior=behavior),tempfile.TemporaryDirectory() as tmp:
                closed=[]
                def campaign(argv,absolute_deadline=None,progress=None):
                    directory=Path(argv[argv.index('--out')+1]);directory.mkdir()
                    try:
                        if behavior=='failed_deadline': progress.update(status='failed',outcomes={'process_failure':1},metrics={})
                        if behavior!='malformed': raise suite.perf.CampaignDeadline()
                        yield
                        return 0
                    finally:
                        closed.append(directory.name)
                        raw={} if behavior=='malformed' else dict(status='finished',aggregate_censored=True,summary=dict(counts={},metrics={}))
                        (directory/'campaign.json').write_text(json.dumps(raw))
                argv=['perf_suite.py','routine','--binary','/bin/true','--only','rewrite','--seconds','10','--out',str(Path(tmp)/'suite')]
                with patch.object(sys,'argv',argv),patch.object(suite.perf,'campaign',campaign,create=True),contextlib.redirect_stdout(io.StringIO()):
                    suite.main()
                report=json.loads((Path(tmp)/'suite/suite.json').read_text())
                self.assertEqual(report['status'],expected)
                self.assertEqual(report['points'][0]['status'],expected)
                self.assertEqual(len(closed),2 if behavior=='malformed' else 1)

    def test_scaling_excludes_configuration_and_oracle_numbers(self):
        allowed=['process.wall_seconds','workload.times_ms.source_delivery','workload.work.applications',
                 'workload.memory_counts.choices','phase.cancel.0.elapsed_ms','phase.source.0.memory.choices',
                 'allocations.total_allocated_bytes','workload.native_peak_rss_estimate_kib',
                 'workload.source_ms','workload.tiny_answer_ms','workload.elapsed_ms',
                 'workload.prepare_ms.0','workload.prepared_drop_ms.1','workload.uses.0.use_ms',
                 'workload.parse_program_ms','workload.generation_ms']
        excluded=['workload.size','workload.seed','workload.max_ticks','phase.source.0.data.first_choice',
                  'workload.answer_count','process.pid','configuration.timeout_seconds','workload.uses.0.answers','workload.prepare_ms.invalid']
        points=[dict(family='x',axis='size',value=v,status='completed',metrics={k:{'median':v} for k in allowed+excluded}) for v in (1,2)]
        self.assertEqual(set(suite.scaling(points)[0]['metrics']),set(allowed))

    def test_scaling_partial_metric_retains_operands_without_ratio(self):
        points=[dict(family='x',axis='size',value=v,status='completed',metrics={
            'process.wall_seconds':{'median':v,'incomplete_or_missing':missing}})
            for v,missing in [(1,0),(2,1)]]
        metric=suite.scaling(points)[0]['metrics']['process.wall_seconds']
        self.assertEqual((metric['small'],metric['large']),(1,2))
        self.assertIsNone(metric['ratio']);self.assertIsNone(metric['exponent'])

    def test_native_selected_sweep_and_budget_censoring(self):
        root=Path(suite.__file__).parents[1]
        with tempfile.TemporaryDirectory() as tmp:
            for seconds,code,repeats in [('15',0,2),('0.01',2,2),('4',2,10000)]:
                out=Path(tmp)/seconds
                run=subprocess.run([sys.executable,suite.__file__,'routine','--binary',str(root/'target/release/examples/measure'),'--only','rewrite','--repeat',str(repeats),'--seconds',seconds,'--out',str(out)],capture_output=True,text=True,timeout=20)
                self.assertEqual(run.returncode,code,run.stderr)
                report=json.loads((out/'suite.json').read_text())
                self.assertEqual(len(report['points']),2)
                if code==0:
                    self.assertEqual(report['status'],'completed')
                    self.assertEqual(report['scaling'][0]['metrics']['workload.work.applications']['ratio'],4)
                    self.assertTrue(all(p['outcomes']=={'completed':2} for p in report['points']))
                    # Samples are atomically persisted immediately before each yield.
                    # File timestamps therefore expose actual native completion order.
                    observed=sorted((path.stat().st_mtime_ns,point['id'],i)
                        for point in report['points'] for i in range(2)
                        for path in [Path(point['campaign']).parent/f'{i}.json'])
                    self.assertEqual([(name,i) for _,name,i in observed],
                                     [(p['id'],i) for i in range(2) for p in report['points']])
                    self.assertTrue(all(json.loads(Path(p['campaign']).read_text())['limits']['warmups']==0 for p in report['points']))
                else:
                    self.assertEqual(report['status'],'censored')
                    if seconds=='0.01':
                        self.assertTrue(all(p['status']=='not_run_budget' for p in report['points']))
                    else:
                        started=[p for p in report['points'] if 'campaign' in p]
                        self.assertTrue(started)
                        self.assertTrue(all(p['status']=='censored' for p in started))
                        self.assertTrue(all(0<p['samples_executed']<repeats for p in started))
                        self.assertTrue(all(json.loads(Path(p['campaign']).read_text())['aggregate_censored'] for p in started))


if __name__=='__main__': unittest.main()
