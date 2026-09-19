#!/usr/bin/env python3
"""Open a 10,000-row owned virtual list in the foreground Sprite pane.

Run with `sprite -e python3 /absolute/path/surface-list-demo.py --check --output /tmp/list.json`.
The result deliberately records no authentication material.
"""
import argparse
import json
import os
import select
import socket
import sys
import traceback
from pathlib import Path

parser = argparse.ArgumentParser()
parser.add_argument('--check', action='store_true')
parser.add_argument('--output', type=Path, default=Path('/tmp/sprite-list-demo.json'))
args = parser.parse_args()
pane = int(os.environ['SPRITE_PANE'])
events, checks = [], []

def connect(message):
    sock = socket.socket(socket.AF_UNIX)
    sock.settimeout(8)
    sock.connect(os.environ['SPRITE_SURFACE_SOCKET'])
    wire = sock.makefile('rwb', buffering=0)
    message = dict(message, pane=message.get('pane', pane))
    write_all(wire, (os.environ['SPRITE_SURFACE_KEY'] + ' ' + json.dumps(message) + '\n').encode())
    return sock, wire, json.loads(wire.readline())

def write_all(wire, payload):
    while payload:
        written = wire.write(payload)
        if not written: raise OSError('surface socket closed during write')
        payload = payload[written:]

def send(wire, message): write_all(wire, (json.dumps(message) + '\n').encode())
def expect(wire, kind, operation=None):
    while True:
        raw = wire.readline()
        if not raw: raise AssertionError('EOF waiting for ' + kind)
        event = json.loads(raw); events.append(event)
        if event['type'] == kind and (operation is None or event.get('operation') == operation): return event
        if event['type'] == 'refused': raise AssertionError(event)

def query(message):
    sock, wire, reply = connect(message)
    wire.close(); sock.close()
    return reply

def close(wire):
    send(wire, {'type': 'close'})
    assert expect(wire, 'closed')['type'] == 'closed'

description = {
    'version': 1, 'root': {'kind': 'virtual_list', 'row_height': 22,
    'font_size': 13, 'font_family': 'system', 'icon_size': 16, 'icon_gap': 6,
    'left_padding': 8, 'right_padding': 8,
    'heading': {'text': 'EXPLORER', 'height': 35, 'font_size': 11},
    'section': {'text': 'PROJECT', 'height': 22, 'font_size': 11, 'font_weight': 'bold'},
    'colors': {'background':'#181818','foreground':'#cccccc','hover':'#2a2d2e',
    'selected':'#04395e','inactive_selected':'#37373d','selected_foreground':'#ffffff',
    'focus':'#0078d4','guide':'#404040','border':'#3f3f46','scrollbar':'#797979'}}}

wire = None
error = None
try:
    capabilities = query({'type':'capabilities','version':1,'owner_pid':os.getpid(),'return_target':'terminal'})
    assert capabilities['type'] == 'capabilities' and 'virtual-list-v1' in capabilities['features']
    checks.append('virtual-list capability discovered')
    open_message = {'type':'open','version':1,'position':'dock','side':'left','size':300,
                    'owner_pid':os.getpid(),'return_target':'terminal','resizable':True,
                    'focus':False,'description':description}
    sock, wire, opened = connect(open_message)
    assert opened['type'] == 'opened'; checks.append('opened virtual-list dock')
    send(wire, {'type':'assets','entries':{'dot':'<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16"><circle cx="8" cy="8" r="6" fill="#61afef"/></svg>'}})
    assert expect(wire, 'applied', 'assets')['operation'] == 'assets'; checks.append('assets acknowledged')
    rows = [{'id':'r%d' % i, 'text':'row %05d' % i, 'indent':0, 'icon':'dot', 'guides':[]} for i in range(10000)]
    send(wire, {'type':'list_rows','revision':1,'rows':rows,'selected':'r5000'})
    assert expect(wire, 'applied', 'list_rows')['revision'] == 1; checks.append('10000 rows acknowledged')
    send(wire, {'type':'list_state','revision':1,'scroll':{'id':'r5000','offset':3},'status':'10000 rows'})
    assert expect(wire, 'applied', 'list_state')['revision'] == 1; checks.append('scroll state acknowledged')
    send(wire, {'type':'list_state','revision':2,'selected':'r1'})
    refused = expect(wire, 'refused'); assert refused['type'] == 'refused'; checks.append('stale revision refused while surface stays open')
    send(wire, {'type':'list_state','revision':1,'selected':'r1'})
    assert expect(wire, 'applied', 'list_state')['revision'] == 1; checks.append('connection remains usable after refusal')
    if args.check:
        close(wire); checks.append('closed after protocol check')
    else:
        print('Virtual list visible; Ctrl-C or stdin/socket EOF closes it.', flush=True)
        sock.settimeout(None)
        socket_eof = False
        while True:
            readable, _, _ = select.select([sock, sys.stdin], [], [])
            if sock in readable:
                raw = wire.readline()
                if not raw:
                    socket_eof = True
                    break
                event = json.loads(raw); events.append(event); print(json.dumps(event), flush=True)
            if sys.stdin in readable and not sys.stdin.readline(): break
        if socket_eof:
            checks.append('surface connection ended at socket EOF')
        else:
            close(wire)
            checks.append('closed after interactive EOF')
except KeyboardInterrupt:
    if wire is not None:
        try:
            close(wire)
            checks.append('closed after interrupt')
        except BaseException:
            error = traceback.format_exc()
            raise
except BaseException:
    error = traceback.format_exc()
    raise
finally:
    result = {'host_pid':os.getppid(),'pane':pane,'checks':checks,'events':events}
    if error: result['error'] = error
    args.output.write_text(json.dumps(result, indent=2))
