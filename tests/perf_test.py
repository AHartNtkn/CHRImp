"""Typed-record and campaign classification checks."""
import importlib.util
import json
from pathlib import Path
import sys
import subprocess
import tempfile
import unittest
sys.path.insert(0,str(Path(__file__).parents[1]/'examples'))
spec=importlib.util.spec_from_file_location('chr_perf',Path(__file__).parents[1]/'examples/perf.py')
perf=importlib.util.module_from_spec(spec);spec.loader.exec_module(perf)


def process(code=0,status='completed'):
    return dict(returncode=code,status=status,descendants_signaled_after_exit=False,group_cleanup_complete=True)


def event(status='COMPLETE'):
    return dict(schema=1,kind='result',data=dict(case='rewrite',size=8,status=status,error=None,source_goal_reached=True,cleanup_done=True,cleanup_in_time=True,memory_counts={},work=dict(answers=1),times_ms=dict(source_delivery=2.,validator=None)))


def config():
    return dict(schema=1,kind='configuration',data=dict(case='rewrite',size=8,detailed=False,diagnostics_feature=False))


class CampaignTests(unittest.TestCase):
    def test_typed_records_ignore_display_and_preserve_null_and_repetition(self):
        row=event()
        parsed=perf.records('display answers=wrong\nmeasurement='+json.dumps(row)+'\nmeasurement='+json.dumps(row))
        self.assertEqual(parsed,[row,row])
        self.assertIsNone(parsed[0]['data']['times_ms']['validator'])
        with self.assertRaises(ValueError): perf.records('measurement={')
        for value in [[], 1, {'schema':2}, {'schema':1,'kind':'result','data':None}]:
            with self.assertRaises(ValueError): perf.records('measurement='+json.dumps(value))

    def test_complete_requires_exactly_one_consistent_result(self):
        self.assertEqual(perf.classify(process(),[config(),event()],'rewrite'),'completed')
        for events in [[],[event(),event()],[event('INCOMPLETE')]]:
            self.assertEqual(perf.classify(process(),events,'rewrite'),'report_error')
        self.assertEqual(perf.classify(process(),[config(),event()],'other'),'report_error')
        self.assertEqual(perf.classify(process(2,'failed'),[config(),event('INCOMPLETE')],'rewrite'),'censored')
        self.assertEqual(perf.classify(process(2,'failed'),[event()],'rewrite'),'report_error')
        self.assertEqual(perf.classify(process(-9,'censored'),[],'rewrite'),'censored')
        self.assertEqual(perf.classify(process(1,'failed'),[dict(kind='error',data={})],'rewrite'),'workload_error')

    def test_contradictory_success_nonfinite_and_failed_cleanup_are_rejected(self):
        for key,value in [('source_goal_reached',False),('cleanup_done',False),('cleanup_in_time',False),('error','broken')]:
            row=event();row['data'][key]=value
            self.assertEqual(perf.classify(process(),[config(),row],'rewrite'),'report_error')
        bad=process(-9,'censored');bad['group_cleanup_complete']=False
        self.assertEqual(perf.classify(bad,[],'rewrite'),'cleanup_failure')
        for number in ['NaN','Infinity','-Infinity','1e400']:
            with self.assertRaises(ValueError):
                perf.records('measurement={"schema":1,"kind":"result","data":{"duration":'+number+'}}')

    def test_censored_and_warmup_samples_never_become_success_timings(self):
        samples=[dict(warmup=False,status='completed',process=process(),records=[event()]),
                 dict(warmup=False,status='censored',records=[]),dict(warmup=True,status='completed',process=process(),records=[event()])]
        report=perf.summary(samples)
        self.assertEqual(report['counts'],{'completed':1,'censored':1})
        self.assertEqual(report['metrics']['workload.times_ms.source_delivery']['n'],1)
        self.assertEqual(report['metrics']['workload.times_ms.validator']['n'],0)
        self.assertIsNone(report['metrics']['workload.times_ms.validator']['median'])
        self.assertEqual(report['metrics']['workload.times_ms.validator']['missing_completed'],1)
        self.assertEqual(report['regression_assessment'],'not_performed')


class NativeRecordTests(unittest.TestCase):
    def test_actual_workload_records_match_independent_small_counts(self):
        binary=Path(__file__).parents[1]/'target/release/examples/measure'
        for case,size,answers,applications in [('rewrite',8,1,16),('duplicate-heads',3,1,6),('notebook-arithmetic-decompose',3,4,None)]:
            for flag in [[],['--detail']]:
                completed=subprocess.run([str(binary),case,str(size),'5000000','5',*flag],capture_output=True,text=True,timeout=15)
                self.assertEqual(completed.returncode,0,completed.stderr)
                events=perf.records(completed.stdout)
                self.assertEqual(perf.classify(process(),events,case),'completed')
                result=next(e['data'] for e in events if e['kind']=='result')
                self.assertEqual(result['work']['answers'],answers)
                if applications is not None: self.assertEqual(result['work']['applications'],applications)
                self.assertTrue(result['cleanup_done'] and result['cleanup_in_time'])
                self.assertEqual(result['times_ms']['validator'] is None,not flag)
                self.assertEqual(set(result['memory_counts']['after_cleanup']),set(result['memory_counts']['before_cleanup']))
                self.assertEqual(result['memory_counts']['after_cleanup']['graph_nodes'],0)

    def test_native_repeats_preserve_samples_and_unavailable_measurements(self):
        with tempfile.TemporaryDirectory() as directory:
            out=Path(directory)/'campaign'
            binary=Path(__file__).parents[1]/'target/release/examples/measure'
            completed=subprocess.run([sys.executable,str(Path(perf.__file__)),'--binary',str(binary),'--repeat','2','--warmup','0','--out',str(out),'--','rewrite','8'],capture_output=True,text=True,timeout=5)
            self.assertEqual(completed.returncode,0,completed.stderr)
            report=json.loads((out/'campaign.json').read_text())
            self.assertEqual(report['summary']['counts'],{'completed':2})
            self.assertEqual(report['summary']['metrics']['workload.work.applications']['median'],16)
            self.assertIsNone(report['summary']['metrics']['workload.times_ms.validator']['median'])
            for index in range(2):
                sample=json.loads((out/f'{index}.json').read_text())
                self.assertEqual(sample['status'],'completed')
                self.assertTrue((out/f'{index}.stdout').is_file())

    def test_exhausted_aggregate_budget_does_not_claim_success(self):
        with tempfile.TemporaryDirectory() as directory:
            out=Path(directory)/'run'
            root=Path(__file__).parents[1]
            completed=subprocess.run([sys.executable,str(root/'examples/perf.py'),'--binary',str(root/'target/release/examples/measure'),'--repeat','2','--warmup','0','--total-seconds','1','--out',str(out),'--','rewrite','8'],capture_output=True,text=True,timeout=5)
            self.assertEqual(completed.returncode,2,completed.stderr)
            report=json.loads((out/'campaign.json').read_text())
            self.assertTrue(report['aggregate_censored'])
            self.assertEqual(report['samples_executed'],0)
            self.assertEqual(report['summary']['counts'],{})


if __name__=='__main__': unittest.main()
