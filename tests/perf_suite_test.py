import importlib.util
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
        self.assertTrue({'size','heads','uses','cadence'}<={p['axis'] for p in deep})
        self.assertTrue(all(p['risk'] for p in deep))

    def test_scaling_preserves_operands_and_censored_gaps(self):
        def point(value,cost,status='completed'):
            return dict(family='x',axis='size',value=value,status=status,metrics={'cost':{'median':cost}})
        r=suite.scaling([point(2,4),point(4,16),point(8,64,'censored'),point(16,256)])
        self.assertEqual(r[0]['metrics']['cost'],dict(small=4,large=16,ratio=4,exponent=2))
        self.assertEqual([p['status'] for p in r],['observed','incomplete_scaling','incomplete_scaling'])
        r=suite.scaling([point(2,0),point(4,1)])[0]['metrics']['cost']
        self.assertIsNone(r['ratio']);self.assertIsNone(r['exponent'])

    def test_admitted_expiry_and_failure_preserve_execution_outcome(self):
        for behavior,expected in [('during','censored'),('after_failure','failed'),('report_error','failed'),('malformed','failed'),('during_failed_report','failed')]:
            with tempfile.TemporaryDirectory() as tmp:
                out=Path(tmp)/'suite';clock=[0.]
                def run(argv,absolute_deadline=None,progress=None):
                    if behavior=='during': raise suite.perf.CampaignDeadline()
                    if behavior=='report_error': raise ValueError('malformed report')
                    if behavior=='during_failed_report':
                        progress.update(status='failed',outcomes={'process_failure':1},metrics={})
                        raise suite.perf.CampaignDeadline()
                    if behavior=='malformed':
                        directory=Path(argv[argv.index('--out')+1]);directory.mkdir()
                        (directory/'campaign.json').write_text('{}')
                        return 0
                    clock[0]=20.
                    return 1
                argv=['perf_suite.py','routine','--binary','/bin/true','--only','rewrite','--seconds','10','--out',str(out)]
                with patch.object(sys,'argv',argv),patch.object(suite.perf,'main',run),patch.object(suite.time,'monotonic',lambda:clock[0]),contextlib.redirect_stdout(io.StringIO()):
                    code=suite.main()
                report=json.loads((out/'suite.json').read_text())
                self.assertEqual(report['status'],expected)
                self.assertEqual(report['points'][0]['status'],expected)
                self.assertEqual(code,2 if expected=='censored' else 1)
                self.assertEqual(report['points'][1]['status'],'not_run_budget')
                self.assertIn('campaign',report['points'][0])

    def test_native_selected_sweep_and_budget_censoring(self):
        root=Path(suite.__file__).parents[1]
        with tempfile.TemporaryDirectory() as tmp:
            for seconds,code in [('15',0),('0.01',2)]:
                out=Path(tmp)/seconds
                run=subprocess.run([sys.executable,suite.__file__,'routine','--binary',str(root/'target/release/examples/measure'),'--only','rewrite','--repeat','2','--seconds',seconds,'--out',str(out)],capture_output=True,text=True,timeout=20)
                self.assertEqual(run.returncode,code,run.stderr)
                report=json.loads((out/'suite.json').read_text())
                self.assertEqual(len(report['points']),2)
                if code==0:
                    self.assertEqual(report['status'],'completed')
                    self.assertEqual(report['scaling'][0]['metrics']['workload.work.applications']['ratio'],4)
                    self.assertTrue(all(p['outcomes']=={'completed':2} for p in report['points']))
                else:
                    self.assertEqual(report['status'],'censored')
                    self.assertTrue(all(p['status']=='not_run_budget' for p in report['points']))


if __name__=='__main__': unittest.main()
