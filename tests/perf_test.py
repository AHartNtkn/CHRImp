"""Typed-record and campaign classification checks."""
import json
from pathlib import Path
import sys
import subprocess
import tempfile
import time
import signal
from unittest.mock import patch
import unittest
sys.path.insert(0,str(Path(__file__).parents[1]/'examples'))
import perf


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

    def test_allocation_traffic_and_live_bytes_reach_comparisons_by_named_phase(self):
        allocation=dict(schema=1,kind='allocations',data=dict(process_live_requested_bytes=32,
            process_peak_requested_bytes=128,phases=[dict(phase='engine',allocations=3,deallocations=2,
            allocated_bytes=192,freed_bytes=160),dict(phase='cleanup',allocations=0,deallocations=1,
            allocated_bytes=0,freed_bytes=32)]))
        sample=dict(warmup=False,status='completed',process=process(),records=[event(),allocation])
        values=perf.sample_values(sample)
        self.assertEqual(values['allocations.process_peak_requested_bytes'],128)
        self.assertEqual(values['allocations.phases.engine.allocated_bytes'],192)
        self.assertEqual(values['allocations.phases.cleanup.freed_bytes'],32)
        self.assertEqual(values['allocations.total_allocated_bytes'],192)
        allocation['data']['phases'].reverse()
        self.assertEqual(values,perf.sample_values(sample))
        self.assertEqual(perf.summary([sample])['metrics']['allocations.total_allocated_bytes']['median'],192)
        for invalid in (-1,True,None):
            allocation['data']['phases'][0]['allocated_bytes']=invalid
            with self.assertRaises(ValueError):perf.sample_values(sample)
        allocation['data']['phases'][0]['allocated_bytes']=0
        allocation['data']['process_live_requested_bytes']=129
        with self.assertRaises(ValueError):perf.sample_values(sample)
        allocation['data']['process_live_requested_bytes']=32
        sample['records'].append(allocation)
        with self.assertRaises(ValueError):perf.sample_values(sample)

    def test_censored_resources_preserve_completed_cleanup_without_inventing_latency(self):
        row=event('INCOMPLETE');row['data'].update(source_goal_reached=False,native_peak_rss_estimate_kib=4096)
        row['data']['times_ms']['cleanup']=12.
        sample=dict(status='censored',process=process(2,'failed'),records=[config(),row])
        values=perf.sample_measurements(sample)
        self.assertEqual(values['workload.native_peak_rss_estimate_kib'],dict(value=4096,complete=True))
        self.assertEqual(values['workload.times_ms.cleanup'],dict(value=12.,complete=True))
        self.assertFalse(values['workload.times_ms.source_delivery']['complete'])
        row['data']['first_answer']={'ms':25.}
        self.assertEqual(perf.sample_measurements(sample)['workload.first_answer.ms'],dict(value=25.,complete=True))
        row['data']['cleanup_done']=False
        self.assertFalse(perf.sample_measurements(sample)['workload.times_ms.cleanup']['complete'])
        sample['process']['group_cleanup_complete']=False
        self.assertEqual(perf.sample_measurements(sample),{})

    def test_censored_and_warmup_samples_never_become_success_timings(self):
        samples=[dict(warmup=False,status='completed',process=process(),records=[event()]),
                 dict(warmup=False,status='censored',records=[]),dict(warmup=True,status='completed',process=process(),records=[event()])]
        report=perf.summary(samples)
        self.assertEqual(report['counts'],{'completed':1,'censored':1})
        self.assertEqual(report['metrics']['workload.times_ms.source_delivery']['n'],1)
        self.assertEqual(report['metrics']['workload.times_ms.validator']['n'],0)
        self.assertIsNone(report['metrics']['workload.times_ms.validator']['median'])
        self.assertEqual(report['metrics']['workload.times_ms.validator']['incomplete_or_missing'],2)
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

    def test_runtime_sessions_preserve_outputs_work_and_owner_release(self):
        binary=Path(__file__).parents[1]/'target/release/examples/measure'
        run=subprocess.run([str(binary),'runtime-sessions','3','100000','5','--closed','2','--retained','2','--rows','2','--batch','1','--replay-every','1','--work','4'],capture_output=True,text=True,timeout=15)
        self.assertEqual(run.returncode,0,run.stderr)
        events=perf.records(run.stdout)
        self.assertEqual(perf.classify(process(),events,'runtime-sessions'),'completed')
        result=next(e['data'] for e in events if e['kind']=='result')
        self.assertEqual(result['finite_applications'],2)
        self.assertGreater(result['source']['replays'],0)
        self.assertEqual(result['after_drop_spools']['descriptors'],0)
        self.assertIn('workload.tiny_answer_ms',perf.sample_values(dict(process=process(),records=events)))
        result['cleanup_complete']=False
        self.assertEqual(perf.classify(process(),events,'runtime-sessions'),'report_error')

    def test_fresh_cases_require_full_work_oracle(self):
        binary=Path(__file__).parents[1]/'target/release/examples/measure'
        for case,applications in [('fresh-contract',26),('fresh-unmerged',18)]:
            args=[str(binary),case,'2','5000000','5','--rows','3','--depth','2','--order','reverse']
            run=subprocess.run(args,capture_output=True,text=True,timeout=10)
            self.assertEqual(run.returncode,0,run.stderr)
            events=perf.records(run.stdout)
            self.assertEqual(perf.classify(process(),events,case),'completed')
            result=next(e['data'] for e in events if e['kind']=='result')
            self.assertEqual(result['work']['applications'],applications)
            self.assertEqual(result['expected']['applications'],applications)
            bad=subprocess.run(args+['--prefix','1'],capture_output=True,text=True,timeout=10)
            self.assertEqual(bad.returncode,1)
            self.assertIn('complete exhaustion',bad.stderr)

    def test_parent_deadline_covers_final_reporting(self):
        binary=Path(__file__).parents[1]/'target/release/examples/measure'
        with tempfile.TemporaryDirectory() as tmp:
            out=Path(tmp)/'run';reporting=[]
            def slow_print(*args):
                reporting.append(True)
                time.sleep(5)
            previous=signal.signal(signal.SIGALRM,perf.deadline_signal)
            try:
                with patch.object(perf,'print',slow_print,create=True),self.assertRaises(perf.CampaignDeadline):
                    perf.main(['--binary',str(binary),'--out',str(out),'--repeat','1','--warmup','0','--','rewrite','8'],absolute_deadline=time.monotonic()+2)
            finally:
                signal.setitimer(signal.ITIMER_REAL,0)
                signal.signal(signal.SIGALRM,previous)
            self.assertTrue(reporting)
            self.assertEqual(json.loads((out/'campaign.json').read_text())['status'],'finished')

    def test_preparation_modes_have_validated_uses_and_censored_outcomes(self):
        binary=Path(__file__).parents[1]/'target/release/examples/measure'
        for case in ['prepare-reuse','prepare-independent']:
            completed=subprocess.run([str(binary),case,'0','5000000','5','--uses','3','--heads','2','--arity','3','--repeats','2','--width','4','--depth','3'],capture_output=True,text=True,timeout=10)
            self.assertEqual(completed.returncode,0,completed.stderr)
            events=perf.records(completed.stdout)
            self.assertEqual(perf.classify(process(),events,case),'completed')
            result=next(e['data'] for e in events if e['kind']=='result')
            self.assertEqual([u['answers'] for u in result['uses']],[1,1,1])
            self.assertEqual([u['applications'] for u in result['uses']],[1,1,1])
            self.assertEqual(len(result['prepare_ms']),1 if case=='prepare-reuse' else 3)
            self.assertIn('workload.uses.0.engine_drop_ms',perf.sample_values(dict(process=process(),records=events)))
            result['uses'][1]['complete']=False
            self.assertEqual(perf.classify(process(),events,case),'report_error')
            censored=subprocess.run([str(binary),case,'1','1','5'],capture_output=True,text=True,timeout=10)
            self.assertEqual(censored.returncode,2,censored.stderr)
            self.assertEqual(perf.classify(process(2),perf.records(censored.stdout),case),'censored')

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
