#!/usr/bin/env python3
"""Externally bound a native command and record Linux process resource use.

python3 examples/supervise.py --out /tmp/run -- target/release/examples/measure rewrite 128
The command is executed directly, without a shell. Build before timing.
"""
import argparse
import json
import math
import os
from pathlib import Path
import platform
import resource
import select
import shutil
import signal
import subprocess
import time

ROOT = Path(__file__).resolve().parents[1]


def signal_group(pid, sig):
    try:
        os.killpg(pid, sig)
        return True
    except ProcessLookupError:
        return False


def group_members(leader):
    """Live members of the managed group; leader must remain unreaped."""
    members = []
    for entry in Path('/proc').iterdir():
        if not entry.name.isdigit() or int(entry.name) == leader:
            continue
        try:
            fields = (entry/'stat').read_text().rsplit(')', 1)[1].split()
            if int(fields[2]) == leader and fields[0] not in ('Z', 'X'):
                members.append(int(entry.name))
        except (FileNotFoundError, ProcessLookupError):
            pass
    return members


def run(command, cwd, stdout, stderr, seconds=40, memory_mib=4096, file_mib=256, grace=1):
    """Serial supervisor. rusage covers leader plus descendants it waited for.

    It is not aggregate process-tree RSS, allocation traffic or exclusive engine
    CPU. Address-space and file limits apply per process/file, not to their sum.
    pidfd readiness avoids busy polling and accidental reaping of other children.
    """
    if not math.isfinite(seconds) or seconds <= 0 or not math.isfinite(grace) or grace < 0:
        raise ValueError('positive finite seconds and nonnegative finite grace required')
    if memory_mib <= 0 or file_mib <= 0:
        raise ValueError('positive resource limits required')

    def limits():
        resource.setrlimit(resource.RLIMIT_AS, (memory_mib * 1024**2,) * 2)
        resource.setrlimit(resource.RLIMIT_FSIZE, (file_mib * 1024**2,) * 2)
        cpu = math.ceil(seconds)
        resource.setrlimit(resource.RLIMIT_CPU, (cpu, cpu + 1))

    # wait4 includes launch/pre-exec work. Record the parent's resident set so
    # a fork-related memory floor remains visible in small native workloads.
    parent_rss_kib = int(Path('/proc/self/statm').read_text().split()[1]) * os.sysconf('SC_PAGE_SIZE') // 1024
    started = time.monotonic()
    process = subprocess.Popen(command, cwd=cwd, stdout=stdout, stderr=stderr,
                               start_new_session=True, preexec_fn=limits)
    deadline = started + seconds
    reason = None
    descendants = []
    cleanup_complete = True
    try:
        fd = os.pidfd_open(process.pid)
        try:
            if not select.select([fd], [], [], max(0, deadline-time.monotonic()))[0]:
                reason = 'wall_deadline'
                signal_group(process.pid, signal.SIGINT)
                if not select.select([fd], [], [], grace)[0]:
                    signal_group(process.pid, signal.SIGKILL)
            # Do not reap the leader until group signaling is finished: its
            # zombie reserves the PGID and prevents signaling a reused identity.
            descendants = group_members(process.pid)
            if descendants:
                signal_group(process.pid, signal.SIGKILL)
                cleanup_deadline = time.monotonic() + 1
                while group_members(process.pid):
                    if time.monotonic() >= cleanup_deadline:
                        cleanup_complete = False
                        break
                    time.sleep(.005)
            _, status, usage = os.wait4(process.pid, 0)
            process.returncode = os.waitstatus_to_exitcode(status)
        finally:
            os.close(fd)
    except BaseException:
        if process.returncode is None:
            signal_group(process.pid, signal.SIGKILL)
            process.wait()
        raise
    elapsed = time.monotonic() - started
    code = process.returncode
    if reason is None:
        if code == -signal.SIGXCPU:
            reason = 'cpu_limit'
        elif code == -signal.SIGXFSZ:
            reason = 'file_size_limit'
    status = 'censored' if reason else ('completed' if code == 0 and not descendants else 'failed')
    return {
        'status': status, 'returncode': code, 'limit_reason': reason,
        'descendants_signaled_after_exit': bool(descendants),
        'descendant_pids': descendants, 'group_cleanup_complete': cleanup_complete,
        'launch_parent_rss_kib': parent_rss_kib,
        'wall_seconds': elapsed, 'user_cpu_seconds': usage.ru_utime,
        'system_cpu_seconds': usage.ru_stime, 'max_rss_kib': usage.ru_maxrss,
        'minor_faults': usage.ru_minflt, 'major_faults': usage.ru_majflt,
        'voluntary_context_switches': usage.ru_nvcsw, 'involuntary_context_switches': usage.ru_nivcsw,
        'filesystem_input_blocks': usage.ru_inblock, 'filesystem_output_blocks': usage.ru_oublock,
        'scope': 'Managed original process group only, not containment for commands that change sessions/groups. Linux wait4 includes launcher/pre-exec work and leader plus waited descendants; max_rss is a high-water mark, not summed tree RSS; CPU includes harness/output/cleanup; block counts are kernel accounting, not byte or syscall counts',
        'limits': {'wall_seconds': seconds, 'interrupt_grace_seconds': grace, 'group_cleanup_grace_seconds': 1,
                   'address_space_mib_per_process': memory_mib, 'file_mib_per_file': file_mib,
                   'cpu_soft_seconds_per_process': math.ceil(seconds), 'cpu_hard_seconds_per_process': math.ceil(seconds)+1},
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--out', type=Path, required=True)
    parser.add_argument('--seconds', type=float, default=40)
    parser.add_argument('--memory-mib', type=int, default=4096)
    parser.add_argument('--file-mib', type=int, default=256)
    parser.add_argument('--measure', action='store_true', help='interpret measure exit 2 as an incomplete workload')
    parser.add_argument('command', nargs=argparse.REMAINDER)
    args = parser.parse_args()
    command = args.command[1:] if args.command[:1] == ['--'] else args.command
    if not command:
        parser.error('a command is required')
    binary = shutil.which(command[0])
    if binary is None:
        parser.error('command executable not found; build it first')
    binary = Path(binary).resolve()
    out = args.out.resolve()
    out.mkdir(parents=True, exist_ok=False)
    metadata = {'schema': 1, 'command': command, 'cwd': str(Path.cwd()),
                'binary': str(binary),
                'host': platform.platform(), 'status': 'starting'}
    path = out / 'run.json'
    path.write_text(json.dumps(metadata, indent=2)+'\n')
    try:
        with (out/'stdout.log').open('w') as stdout, (out/'stderr.log').open('w') as stderr:
            result = run(command, Path.cwd(), stdout, stderr, args.seconds, args.memory_mib, args.file_mib)
    except (OSError, ValueError, subprocess.SubprocessError) as error:
        metadata.update(status='supervisor_error', error=str(error))
        path.write_text(json.dumps(metadata, indent=2)+'\n')
        raise
    if args.measure and result['returncode'] == 2 and result['limit_reason'] is None and not result['descendants_signaled_after_exit']:
        result.update(status='censored', limit_reason='workload_incomplete')
    metadata.update(result)
    path.write_text(json.dumps(metadata, indent=2)+'\n')
    print(json.dumps(metadata))
    return {'completed': 0, 'censored': 2, 'failed': 1}[result['status']]


if __name__ == '__main__':
    raise SystemExit(main())
