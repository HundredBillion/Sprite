"""Fold eu-stack output: drop idle (poll-wait) samples, count frames (inclusive) and leaf."""
import re, subprocess, sys, collections
text = open(sys.argv[1]).read()
samples = [s for s in text.split('===') if 'TID' in s]
busy = [s for s in samples if 'Poller4wait' not in s and 'epoll_wait' not in s]
def frames(s):
    return [m.group(1) for m in re.finditer(r'^#\d+\s+0x[0-9a-f]+\s+(\S+)', s, re.M)]
names = sorted({f for s in busy for f in frames(s)})
dem = subprocess.run(['c++filt'], input='\n'.join(names), capture_output=True, text=True).stdout.split('\n')
D = dict(zip(names, dem))
def strip_generics(n):
    # Remove generic args (a '<' right after an identifier char), keep <Type>:: impl prefixes.
    out, i = [], 0
    while i < len(n):
        if n[i] == '<' and out and (out[-1].isalnum() or out[-1] in '_'):
            depth = 0
            while i < len(n):
                depth += n[i] == '<'; depth -= n[i] == '>'; i += 1
                if depth == 0: break
            continue
        out.append(n[i]); i += 1
    return ''.join(out)
def short(n):
    n = re.sub(r'\[[0-9a-f]{16}\]', '', D.get(n, n))
    n = strip_generics(n).replace('{closure#', '{c')
    return n[:170]
incl, leaf = collections.Counter(), collections.Counter()
for s in busy:
    f = [short(x) for x in frames(s)]
    if f: leaf[f[0]] += 1
    for x in set(f): incl[x] += 1
print(f'samples {len(samples)} busy {len(busy)}')
k = int(sys.argv[2]) if len(sys.argv) > 2 else 60
print('--- inclusive'); [print(f'{c:4d} {100*c/len(busy):5.1f}% {n}') for n, c in incl.most_common(k)]
print('--- leaf'); [print(f'{c:4d} {n}') for n, c in leaf.most_common(25)]
