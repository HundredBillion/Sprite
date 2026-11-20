"""Compare Sprite builds on the #159 workload.

usage: measure.py OUTDIR ROUNDS label=/path/to/sprite [label=/path ...] [-- phase specs]
       measure.py OUTDIR 1 label=/path --sample 200 -- full_40hz:40:20

Runs each binary once per round, order reversed on alternate rounds. Process CPU
(%) of the terminal only, averaged over complete 250 ms intervals per phase.
Waits for the CPU package below 78 C for 3 s before each run; stops at 95 C or
off AC. Restores focus to the window that had it. Never touches other windows.
With --sample N, takes N main-thread stacks with eu-stack during the run.
"""
import ctypes, json, os, statistics, subprocess as S, sys, time
from pathlib import Path

HERE = Path(__file__).parent
args = sys.argv[1:]
phases = []
if '--' in args:
    i = args.index('--'); phases = args[i + 1:]; args = args[:i]
samples = 0
if '--sample' in args:
    i = args.index('--sample'); samples = int(args[i + 1]); del args[i:i + 2]
out = Path(args[0]); rounds = int(args[1])
builds = [a.split('=', 1) for a in args[2:]]
out.mkdir(parents=True, exist_ok=True)
config = out / 'sprite.toml'
config.write_text('[font]\nfamily = "JetBrainsMono Nerd Font"\nsize = 12\nline_height = 1.1666667\n')
hz = os.sysconf('SC_CLK_TCK')

def cpu(pid):
    s = Path(f'/proc/{pid}/stat').read_text().rsplit(')', 1)[1].split()
    return (int(s[11]) + int(s[12])) / hz
def temperature():
    return max(int(t.read_text()) / 1000 for h in Path('/sys/class/hwmon').glob('hwmon*')
               if (h / 'name').read_text().strip() == 'coretemp' for t in h.glob('temp*_input'))
hot = [0]
def check():
    # Stop on 95 C held for three checks in a row, any 98 C, or off AC.
    # Single bursts to the mid-90s happen on this laptop at near idle.
    t = temperature()
    hot[0] = hot[0] + 1 if t >= 95 else 0
    if hot[0] >= 3 or t >= 98 or Path('/sys/class/power_supply/AC/online').read_text().strip() != '1':
        raise RuntimeError(f'thermal/AC guard: {t}')
    return t
def hypr(*a): return S.check_output(['hyprctl', *a], text=True)
def dispatch(expr): return hypr('dispatch', expr)

libc = ctypes.CDLL(None)
def allow_trace():  # PR_SET_PTRACER, PR_SET_PTRACER_ANY: only this test process
    libc.prctl(0x59616d61, ctypes.c_ulong(-1), 0, 0, 0)

original = json.loads(hypr('activewindow', '-j')).get('address')
results = []
try:
    for r in range(rounds):
        order = builds if r % 2 == 0 else builds[::-1]
        for label, binary in order:
            deadline = time.monotonic() + 120; stable = None
            while time.monotonic() < deadline:
                t = check()
                stable = (stable or time.monotonic()) if t < 78 else None
                if stable and time.monotonic() - stable >= 3: break
                time.sleep(.25)
            else: raise RuntimeError('no cool window')
            d = out / f'r{r:02d}-{label}'; d.mkdir(); (d / 'phase').write_text('startup')
            with (d / 'log').open('w') as log:
                proc = S.Popen([binary, '--config', str(config), '-e', 'python3', str(HERE / 'workload.py'), str(d), *phases],
                               stdout=log, stderr=log, preexec_fn=allow_trace if samples else None)
                try:
                    deadline = time.monotonic() + 15
                    while time.monotonic() < deadline:
                        check(); w = next((c for c in json.loads(hypr('clients', '-j')) if c['pid'] == proc.pid), None)
                        if w: break
                        if proc.poll() is not None: raise RuntimeError('terminal exited')
                        time.sleep(.1)
                    else: raise RuntimeError('no test window')
                    addr = w['address']
                    if not w['floating']: dispatch(f'hl.dsp.window.float({{action = "toggle", window = "address:{addr}"}})')
                    dispatch(f'hl.dsp.window.resize({{x = 900, y = 700, relative = false, window = "address:{addr}"}})')
                    time.sleep(.7)
                    w = next(c for c in json.loads(hypr('clients', '-j')) if c['pid'] == proc.pid)
                    assert abs(w['size'][0]-900) <= 2 and abs(w['size'][1]-700) <= 2, w['size']; row_size = w['size']
                    row = {'label': label, 'binary': binary, 'round': r, 'pid': proc.pid, 'size': row_size, 'samples': []}; results.append(row)
                    sampler = None
                    if samples:
                        sampler = S.Popen(['bash', '-c', f'sleep 2; for i in $(seq {samples}); do eu-stack -1 -p {proc.pid} 2>&1; echo ===; done'],
                                          stdout=(d / 'stacks.txt').open('w'), stderr=S.STDOUT)
                    (d / 'go').touch(); before = cpu(proc.pid); old = time.monotonic(); deadline = old + 120
                    while time.monotonic() < deadline:
                        phase = (d / 'phase').read_text(); time.sleep(.25)
                        now = time.monotonic(); after = cpu(proc.pid); temp = check()
                        if phase == (d / 'phase').read_text() and phase not in ('startup', 'done'):
                            row['samples'].append({'phase': phase, 'cpu': 100 * (after - before) / (now - old), 'C': temp})
                        before, old = after, now
                        if (d / 'phase').read_text() == 'done': break
                    else: raise RuntimeError('workload deadline')
                    if sampler: sampler.terminate(); sampler.wait()
                    row['cells'] = json.loads((d / 'dimensions.json').read_text())
                    names = list(dict.fromkeys(s['phase'] for s in row['samples']))
                    row['means'] = {n: round(statistics.mean(s['cpu'] for s in row['samples'] if s['phase'] == n), 2) for n in names}
                    print(json.dumps({k: v for k, v in row.items() if k not in ('samples', 'binary')}), flush=True)
                finally:
                    if proc.poll() is None: proc.terminate()
                    try: proc.wait(timeout=3)
                    except S.TimeoutExpired: proc.kill(); proc.wait()
                    (out / 'results.json').write_text(json.dumps(results, indent=1))
                    if original: dispatch(f'hl.dsp.focus({{window = "address:{original}"}})')
            time.sleep(1)
finally:
    if original: dispatch(f'hl.dsp.focus({{window = "address:{original}"}})')
labels = list(dict.fromkeys(l for l, _ in builds))
summary = {}
for l in labels:
    rows = [r for r in results if r['label'] == l and 'means' in r]
    if rows:
        summary[l] = {p: [r['means'].get(p) for r in rows] for p in rows[0]['means']}
(out / 'summary.json').write_text(json.dumps(summary, indent=1)); print(json.dumps(summary, indent=1))
