import argparse,base64,json,os,pathlib,socket,subprocess,sys,tempfile,time,zlib

def child(directory):
    server=socket.socket(socket.AF_UNIX); server.bind(str(directory/'child.sock')); server.listen(1)
    conn,_=server.accept(); file=conn.makefile('rwb',buffering=0)
    file.write((json.dumps({'pane':os.environ.get('SPRITE_PANE')})+'\n').encode())
    def image(i,color):
        pixels=bytes([color,100,150,255])*(512*512)
        data=base64.b64encode(zlib.compress(pixels))
        os.write(1,(b'\x1b[H' if i == 1 else b'\x1b[30;1H')+b'\x1b_Ga=T,f=32,s=512,v=512,i='+str(i).encode()+b',t=d,o=z;'+data+b'\x1b\\')
    for line in file:
        command=json.loads(line)
        if command=='images':
            image(1,50)
            for i in range(60):
                image(42,80+i); time.sleep(.12)
        elif command=='delete': os.write(1,b'\x1b_Ga=d,d=I,i=42\x1b\\')
        elif command=='delete_all': os.write(1,b'\x1b_Ga=d,d=A\x1b\\\x1b[2J\x1b[H')
        elif command=='wake': os.write(1,b'\x1b[2J\x1b[Hwake '+str(time.time_ns()).encode())
        elif command=='exit': break
        file.write((str(time.time_ns())+'\n').encode())
    conn.close();server.close()

def sample(pid):
    root=pathlib.Path(f'/proc/{pid}')
    stat=(root/'stat').read_text().split(') ',1)[1].split()
    result={'ticks':int(stat[11])+int(stat[12]),'threads':len(list((root/'task').iterdir())), 'fds':len(list((root/'fd').iterdir()))}
    status=(root/'status').read_text().splitlines()
    result['rss_kib']=int(next(x for x in status if x.startswith('VmRSS:')).split()[1])
    clients={}
    for path in (root/'fdinfo').iterdir():
        try: fields=dict(line.split(':',1) for line in path.read_text().splitlines() if ':' in line)
        except OSError: continue
        if 'drm-client-id' in fields:
            clients[fields['drm-client-id']]={k:int(v.split()[0]) for k,v in fields.items() if k.startswith('drm-total-') or k.startswith('drm-resident-')}
    result['gpu_kib']={key:sum(c.get(key,0) for c in clients.values()) for key in {k for c in clients.values() for k in c}}
    return result

def run(binary,backend,output,seconds):
    with tempfile.TemporaryDirectory(prefix='sprite-141-') as tmp:
        directory=pathlib.Path(tmp); config=directory/'config.toml'
        config.write_text('[cursor]\nblink = false\n[shell]\nprogram = "/usr/bin/sleep"\nargs = ["600"]\n')
        env=os.environ.copy();env['SPRITE_LOG']='sprite_app::startup=debug,sprite_app::perf=debug'
        if backend=='x11':env.pop('WAYLAND_DISPLAY',None)
        logpath=output.with_suffix('.frames.log'); log=logpath.open('wb')
        proc=subprocess.Popen([binary,'--config',str(config),'-e',sys.executable,__file__,'--child',tmp],env=env,stdout=log,stderr=log)
        conn=socket.socket(socket.AF_UNIX);conn.settimeout(30)
        result={'binary':binary,'backend':backend,'pid':proc.pid,'seconds':seconds}
        def event(name,data):
            result[name]=data; print(name,json.dumps(data),flush=True)
            output.write_text(json.dumps(result,indent=2)+'\n')
        def cli(*args):
            return json.loads(subprocess.check_output([binary,'panes',*args],env=env,text=True))
        def send(cmd):
            conn.sendall((json.dumps(cmd)+'\n').encode()); return int(stream.readline())
        def idle(name):
            time.sleep(2.5); before=sample(proc.pid); start=time.monotonic(); offset=logpath.stat().st_size
            time.sleep(seconds); elapsed=time.monotonic()-start;after=sample(proc.pid)
            lines=logpath.read_bytes()[offset:]
            event(name,{'cpu_percent':100*(after['ticks']-before['ticks'])/os.sysconf('SC_CLK_TCK')/elapsed,'frames':lines.count(b'[SPRITE141_FRAME]'),'presents':lines.count(b'[SPRITE141_PRESENT]'),'resources':after})
        try:
            deadline=time.monotonic()+45
            while not (directory/'child.sock').exists():
                if proc.poll() is not None:raise RuntimeError(logpath.read_text())
                if time.monotonic()>deadline:raise TimeoutError('child ready')
                time.sleep(.1)
            conn.connect(str(directory/'child.sock'));stream=conn.makefile('rb')
            pane=json.loads(stream.readline())['pane'];event('pane',pane)
            deadline=time.monotonic()+15
            while True:
                clients=json.loads(subprocess.check_output(['hyprctl','clients','-j']))
                windows=[w for w in clients if w['pid']==proc.pid]
                if windows:break
                if time.monotonic()>deadline:raise TimeoutError('desktop window')
                time.sleep(.1)
            w=windows[0];event('window',{k:w[k] for k in ['address','xwayland','workspace','size','visible']})
            if b'[SPRITE141_FRAME]' not in logpath.read_bytes():
                raise RuntimeError('build with instrumentation.patch before measuring')
            idle('visible_idle')
            subprocess.run(['hyprctl','dispatch',f'hl.dsp.window.move({{window = "address:{w["address"]}", workspace = "special:sprite141", follow = false}})'],check=True,stdout=subprocess.DEVNULL)
            idle('hidden_idle')
            subprocess.run(['hyprctl','dispatch',f'hl.dsp.window.move({{window = "address:{w["address"]}", workspace = "{w["workspace"]["id"]}", follow = false}})'],check=True,stdout=subprocess.DEVNULL)
            time.sleep(3)
            wakes=[]
            for _ in range(8):
                time.sleep(2.5); offset=logpath.stat().st_size; start=time.time_ns(); send('wake')
                deadline=time.monotonic()+5
                while True:
                    presents=[int(line.split()[1]) for line in logpath.read_bytes()[offset:].splitlines() if line.startswith(b'[SPRITE141_PRESENT]')]
                    if presents:break
                    if time.monotonic()>deadline:raise TimeoutError('wake present')
                    time.sleep(.001)
                wakes.append((presents[0]-start)/1e6)
            event('wake_to_submission_ms',wakes)
            time.sleep(2);event('before_images',sample(proc.pid));send('images');time.sleep(3);event('after_60_images',sample(proc.pid))
            current=next(win for win in json.loads(subprocess.check_output(['hyprctl','clients','-j'])) if win['pid']==proc.pid)
            x,y=current['at']; width,height=current['size']
            subprocess.run(['grim','-g',f'{x},{y} {width}x{height}',str(output.with_suffix('.images.png'))],check=True)
            send('delete');time.sleep(3);event('after_delete',sample(proc.pid));send('delete_all');time.sleep(3);event('after_delete_all',sample(proc.pid))
            pane_before=sample(proc.pid);created=[]
            for _ in range(4):created.append(cli('create','--beside',pane,'--side','right')['result']['id'])
            time.sleep(4);event('with_4_extra_panes',{'resources':sample(proc.pid),'baseline':pane_before})
            for p in created:cli('close','--pane',p)
            time.sleep(5);event('after_close_panes',sample(proc.pid))
            for _ in range(5):
                extra=[cli('create','--beside',pane,'--side','right')['result']['id'] for _ in range(4)]
                time.sleep(.6)
                for extra_pane in extra:cli('close','--pane',extra_pane)
                time.sleep(.6)
            time.sleep(3);event('after_5_more_pane_cycles',sample(proc.pid))
            cli('close','--pane',pane);proc.wait(timeout=15)
            assert result['visible_idle']['frames'] <= 1, 'visible idle frame delivery must pause'
            assert result['hidden_idle']['frames'] <= 1, 'hidden idle frame delivery must pause'
        finally:
            conn.close()
            if proc.poll() is None:
                proc.terminate()
                try:proc.wait(timeout=5)
                except subprocess.TimeoutExpired:proc.kill();proc.wait()
            log.close()

if __name__=='__main__':
    if len(sys.argv)>1 and sys.argv[1]=='--child':child(pathlib.Path(sys.argv[2]))
    else:
        parser=argparse.ArgumentParser();parser.add_argument('binary');parser.add_argument('backend',choices=['wayland','x11']);parser.add_argument('output',type=pathlib.Path);parser.add_argument('--seconds',type=float,default=15)
        args=parser.parse_args();run(os.path.abspath(args.binary),args.backend,args.output,args.seconds)
