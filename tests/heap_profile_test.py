import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
sys.path.insert(0,str(Path(__file__).parents[1]/'examples'))
import heap_profile as heap


class HeapParsingTests(unittest.TestCase):
    def test_zero_peak_sites_preserve_total_and_invalid_weights_fail(self):
        text,total=heap.heap_stacks('main;a 0\nmain;b 32\nmain;c 16\n')
        self.assertEqual(total,48);self.assertEqual(text,'main;b 32\nmain;c 16\n')
        with self.assertRaises(ValueError):heap.heap_stacks('main -1')
        with self.assertRaises(ValueError):heap.heap_stacks('main 0')
        self.assertEqual(heap.allocation_count('calls to allocation functions: 123 (12/s)\n'),123)
        with self.assertRaises(ValueError):heap.allocation_count('unrecognized report')

    def test_failed_cleanup_takes_precedence_over_deadline(self):
        process=dict(status='censored',returncode=-9,group_cleanup_complete=False,descendants_signaled_after_exit=True)
        self.assertEqual(heap.stage_status(process),'failed')
        process['group_cleanup_complete']=True
        self.assertEqual(heap.stage_status(process),'censored')
        process.update(status='completed',returncode=0)
        self.assertEqual(heap.stage_status(process),'failed')

    def test_timeline_units_and_snapshots(self):
        data='snapshot=0\ntime=0\nmem_heap_B=12\nsnapshot=1\ntime=0.02\nmem_heap_B=4\n'
        self.assertEqual(heap.timeline(data),[dict(seconds=0.,heap_bytes=12),dict(seconds=.02,heap_bytes=4)])
        for invalid in ['snapshot=0\ntime=0\n','snapshot=0\ntime=NaN\nmem_heap_B=0\n','snapshot=0\ntime=0\nmem_heap_B=-1\n']:
            with self.assertRaises(ValueError):heap.timeline(invalid)


@unittest.skipUnless(os.environ.get('CHR_HEAPTRACK_PREFIX'),'set CHR_HEAPTRACK_PREFIX for native heap calibration')
class NativeHeapCalibration(unittest.TestCase):
    def test_known_allocation_stack_and_actual_failure_status(self):
        with tempfile.TemporaryDirectory() as temporary:
            root=Path(temporary);source=root/'calibration.c';binary=root/'calibration'
            source.write_text('''#include <stdlib.h>
#include <string.h>
#include <unistd.h>
__attribute__((noinline)) void allocate_control(void) {
    volatile char *a=malloc(1048576), *b=calloc(1,524288);
    a[0]=1;b[0]=2;
    a=realloc((void*)a,2097152);a[0]=3;
    usleep(50000);
    free((void*)a);free((void*)b);
}
int main(int argc,char **argv) {
    if(argc>1 && !strcmp(argv[1],"fail"))return 7;
    if(argc>1 && !strcmp(argv[1],"truncated")){write(1,"measurement={",13);usleep(5000000);return 0;}
    allocate_control();return 0;
}
''')
            subprocess.run(['cc','-g','-O0',str(source),'-o',str(binary)],check=True,timeout=30)
            command=[sys.executable,heap.__file__,'--prefix',os.environ['CHR_HEAPTRACK_PREFIX'],'--binary',str(binary),'--cli','--seconds','5']
            out=root/'profile'
            result=subprocess.run(command+['--out',str(out),'--','active'],capture_output=True,text=True,timeout=30)
            self.assertEqual(result.returncode,0,result.stderr)
            metadata=json.loads((out/'heap.json').read_text())
            self.assertTrue(metadata['profile_validated'])
            rows=(out/'allocations.folded').read_text().splitlines()
            control=sum(int(row.rsplit(' ',1)[1]) for row in rows if 'allocate_control' in row)
            self.assertEqual(control,3)
            peak=sum(int(row.rsplit(' ',1)[1]) for row in (out/'peak.folded').read_text().splitlines() if 'allocate_control' in row)
            self.assertEqual(peak,2097152+524288)
            self.assertGreaterEqual(metadata['timeline_sampled_peak_bytes'],2097152+524288)
            duration=json.loads((out/'timeline.json').read_text())[-1]['seconds']
            self.assertGreaterEqual(duration,.04);self.assertLess(duration,5)
            failed=root/'failed'
            result=subprocess.run(command+['--out',str(failed),'--','fail'],capture_output=True,text=True,timeout=15)
            self.assertNotEqual(result.returncode,0)
            metadata=json.loads((failed/'heap.json').read_text())
            self.assertEqual(metadata['status'],'failed')
            self.assertEqual(metadata['process_resources']['returncode'],7)
            self.assertFalse(metadata['profile_validated'])
            limited=root/'analysis-limited'
            result=subprocess.run(command+['--analysis-seconds','0.001','--out',str(limited),'--','active'],capture_output=True,text=True,timeout=15)
            self.assertEqual(result.returncode,2,result.stderr)
            metadata=json.loads((limited/'heap.json').read_text())
            self.assertEqual(metadata['status'],'completed')
            self.assertEqual(metadata['analysis_status'],'censored')
            self.assertFalse(metadata['profile_validated'])
            truncated=root/'truncated'
            raw_command=[arg for arg in command if arg!='--cli']
            result=subprocess.run(raw_command+['--seconds','0.05','--out',str(truncated),'--','truncated'],capture_output=True,text=True,timeout=15)
            metadata=json.loads((truncated/'heap.json').read_text())
            self.assertEqual(metadata['status'],'censored')
            self.assertEqual(metadata['process_resources']['status'],'censored')
            self.assertIn('report_error',metadata)



if __name__=='__main__':unittest.main()
