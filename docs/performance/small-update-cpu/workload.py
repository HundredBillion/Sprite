"""Synthetic #159 workload (plus a scroll phase): 28 static rows of 79 characters and one status line.

Same bytes as ~/.local/state/sprite-cpu-20261107/workload.py. Phases are passed
as argv[2:] as name:rate:seconds; "clear" before a phase name clears the screen.
"""
import json, os, sys, time
from pathlib import Path

p = Path(sys.argv[1])
phases = [a.split(':') for a in sys.argv[2:]] or [
    ['quiet_full', '0', '3'], ['full_40hz', '40', '3'], ['full_5hz', '5', '3'],
    ['clear+sparse_40hz', '40', '3'], ['quiet_sparse', '0', '3'], ['scroll_40hz', '40', '3']]
while not (p / 'go').exists():
    time.sleep(.05)
(p / 'dimensions.json').write_text(json.dumps(list(os.get_terminal_size())))
def emit(s): sys.stdout.write(s); sys.stdout.flush()
emit('\033[?1049h\033[?25l\033[2J')
for row in range(1, 29):
    emit(f'\033[{row};1H' + ('Static row %02d ' % row + 'abcdefghij ' * 8)[:79])
for name, rate, seconds in phases:
    rate, seconds = int(rate), float(seconds)
    if name.startswith('clear+'):
        name = name[6:]; emit('\033[2J')
    (p / 'phase').write_text(name); start = time.monotonic()
    for n in range(int(seconds * 40)):
        if rate and name.startswith('scroll') and n % (40 // rate) == 0:
            # A whole-screen scroll each update: every row on screen moves.
            emit(f'\033[999;1H\n' + ('Scrolled line %05d ' % n + 'abcdefghij ' * 8)[:79])
        elif rate and n % (40 // rate) == 0:
            emit(f'\033[29;1H\033[2K\033[38;2;120;180;220m{"|/-X"[(n // (40 // rate)) % 4]} Working on synthetic task\033[0m')
        time.sleep(max(0, start + (n + 1) / 40 - time.monotonic()))
(p / 'phase').write_text('done'); time.sleep(1)
