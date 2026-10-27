import json, os, signal, subprocess, time
binary = os.path.abspath('target/release/sprite')
results = []
for trial in range(3):
    for workload, command in [('idle', 'sleep 20'), ('output', 'yes 0123456789 | head -c 10485760; sleep 20')]:
        for mode, log_filter in [('default', 'sprite_app=warn'), ('detailed', 'sprite_app=debug')]:
            env = os.environ.copy(); env['SPRITE_LOG'] = log_filter
            started = time.monotonic()
            proc = subprocess.Popen([binary, '-e', '/bin/sh', '-c', command], stdout=subprocess.DEVNULL, stderr=subprocess.PIPE, env=env)
            time.sleep(4)
            state = proc.poll()
            sample = subprocess.run(['ps', '-p', str(proc.pid), '-o', 'time=,rss='], text=True, capture_output=True).stdout.strip() if state is None else ''
            if state is None:
                proc.terminate()
                try: proc.wait(timeout=3)
                except subprocess.TimeoutExpired: proc.kill(); proc.wait()
            stderr = proc.stderr.read().decode(errors='replace')[-1000:]
            results.append({'trial': trial + 1, 'workload': workload, 'mode': mode, 'sample': sample, 'exited_early': state, 'elapsed_seconds': round(time.monotonic()-started, 3), 'stderr_tail': stderr})
            print(json.dumps(results[-1]), flush=True)
with open('/tmp/sprite-log-measure.json', 'w') as output: json.dump(results, output, indent=2)
