#!/usr/bin/env python3
"""Exercise owned Surface focus and cleanup through a real Sprite window.

Run as its foreground command: sprite -e python3 /absolute/path/to/this/script
--output /tmp/surface-owner-result.json. Keep the test window active. The output records each passed check.
"""

import argparse
import json
import os
import select
import signal
import socket
import subprocess
import sys
import traceback
from pathlib import Path


parser = argparse.ArgumentParser(description='Check owned Surface lifecycle from a foreground Sprite pane.')
parser.add_argument('--output', type=Path, required=True)
log = parser.parse_args().output
pane = int(os.environ['SPRITE_PANE'])
checks = []


def connect(message):
    s = socket.socket(socket.AF_UNIX)
    s.settimeout(8)
    s.connect(os.environ['SPRITE_SURFACE_SOCKET'])
    f = s.makefile('rwb', buffering=0)
    f.write((os.environ['SPRITE_SURFACE_KEY'] + ' ' + json.dumps(dict(message, pane=message.get('pane', pane))) + '\n').encode())
    return (s, f, json.loads(f.readline()))


def send(f, message):
    f.write((json.dumps(message) + '\n').encode())


def expect(f, kind):
    while True:
        raw = f.readline()
        if not raw:
            raise AssertionError('EOF waiting for ' + kind)
        event = json.loads(raw)
        if event['type'] == kind:
            return event
        if event['type'] == 'refused':
            raise AssertionError(event)


def assert_no_focus_change(s, f, name, allow_eof=False):
    # Focus notifications arrive at frame boundaries; wait for delayed changes.
    import time
    deadline = time.monotonic() + 1
    while True:
        remaining = deadline - time.monotonic()
        if remaining <= 0 or not select.select([s], [], [], remaining)[0]:
            break
        raw = f.readline()
        if not raw:
            if allow_eof:
                break
            raise AssertionError('unexpected EOF: ' + name)
        event = json.loads(raw)
        assert event['type'] not in ('focus', 'blur', 'closed', 'input', 'event'), (name, event)
    checks.append(name)


def query(message):
    s, f, result = connect(message)
    f.close()
    s.close()
    return result


def check(condition, name):
    assert condition, name
    checks.append(name)


def op(position, **kw):
    return dict(
        type='open', version=1, position=position,
        description={
            'version': 1,
            'root': {'kind': 'text', 'text': 'Isolated native Surface acceptance'},
        },
        **kw,
    )


try:
    caps = dict(type='capabilities', version=1, owner_pid=os.getpid(), return_target='terminal')
    check(query(caps)['type'] == 'capabilities', 'terminal owner eligible')
    sf, ff, opened = connect(op('fill', owner_pid=os.getpid()))
    check(opened['type'] == 'opened', 'registered fill opens')
    fid = opened['surface']
    expect(ff, 'focus')
    check(query(caps)['type'] == 'refused', 'terminal target refused under fill')
    caps['return_target'] = fid
    check(query(caps)['eligible'], 'registered fill owner eligible')
    sd, fd, opened = connect(op('dock', owner_pid=os.getpid(), return_target=fid, resizable=True))
    check(opened['type'] == 'opened', 'owned dock opens')
    did = opened['surface']
    expect(fd, 'focus')
    expect(ff, 'blur')
    check(query(op('dock', owner_pid=os.getpid(), return_target=fid, resizable=True))['type'] == 'refused', 'occupied owned dock refused')
    assert_no_focus_change(sd, fd, 'occupied refusal preserves incumbent focus')
    caps['return_target'] = did
    check(query(caps)['type'] == 'refused', 'dock cannot be an editor return target')
    caps['return_target'] = fid
    check(query(dict(type='focus', pane=pane + 1000, target=did))['type'] == 'refused', 'unknown-pane focus refused')
    send(fd, {'type': 'close'})
    expect(fd, 'closed')
    expect(ff, 'focus')
    checks.append('closing focused dock returns focus to registered fill')
    fd.close()
    sd.close()
    sd, fd, opened = connect(op('dock', focus=False, owner_pid=os.getpid(), return_target=fid, resizable=True))
    check(opened['type'] == 'opened', 'unfocused owned dock opens')
    check(query(dict(type='focus', target='terminal'))['type'] == 'focused',
          'terminal takes focus before blurred close')
    expect(ff, 'blur')
    send(fd, {'type': 'close'})
    expect(fd, 'closed')
    assert_no_focus_change(sf, ff, 'closing blurred dock does not steal terminal focus')
    check(query(dict(type='focus', target=fid))['type'] == 'focused',
          'fill remains focusable after blurred dock closes')
    expect(ff, 'focus')
    fd.close()
    sd.close()
    sd, fd, opened = connect(op('dock', focus=False, owner_pid=os.getpid(), return_target=fid, resizable=True))
    check(opened['type'] == 'opened', 'dependent dock reopens')
    send(ff, {'type': 'close'})
    expect(fd, 'closed')
    expect(ff, 'closed')
    checks.append('closing fill closes dependent dock')
    fd.close()
    sd.close()
    ff.close()
    sf.close()
    signal.signal(signal.SIGTTOU, signal.SIG_IGN)
    child = subprocess.Popen([sys.executable, '-c', 'import signal; signal.pause()'], preexec_fn=os.setpgrp)
    try:
        os.tcsetpgrp(0, child.pid)
        sd, fd, opened = connect(op('dock', owner_pid=child.pid, return_target='terminal', resizable=True))
        check(opened['type'] == 'opened', 'foreground child owns dock')
        did = opened['surface']
        expect(fd, 'focus')
        os.kill(child.pid, signal.SIGSTOP)
        os.waitpid(child.pid, os.WUNTRACED)
        os.tcsetpgrp(0, os.getpgrp())
        check(query(dict(type='focus', target=did))['type'] == 'refused', 'suspended background owner refused focus')
        draining = []
        while True:
            event = json.loads(fd.readline())
            draining.append(event['type'])
            if event['type'] == 'closed':
                break
        check(not any((kind in draining for kind in ('focus', 'blur', 'input', 'event'))), 'invalidated dock receives no stale event')
        assert_no_focus_change(sd, fd, 'invalidated dock stays silent after close', allow_eof=True)
        fd.close()
        sd.close()
    finally:
        os.tcsetpgrp(0, os.getpgrp())
        child.kill()
        child.wait()
    log.write_text(json.dumps({'passed': checks}, indent=2))
except BaseException:
    log.write_text(json.dumps({'passed': checks, 'error': traceback.format_exc()}, indent=2))
    raise
