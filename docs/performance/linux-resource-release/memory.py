import ctypes,importlib.util,json,os,pathlib,socket,subprocess,sys,tempfile,time
root=pathlib.Path(__file__).resolve().parents[3]
spec=importlib.util.spec_from_file_location('measure',root/'docs/performance/linux-resource-release/measure.py');m=importlib.util.module_from_spec(spec);spec.loader.exec_module(m)
libc=ctypes.CDLL(None)
def debug_parent():
    if libc.prctl(0x59616d61,os.getppid(),0,0,0): raise OSError('PR_SET_PTRACER')
with tempfile.TemporaryDirectory(prefix='sprite141-memory-') as tmp:
 d=pathlib.Path(tmp); config=d/'config.toml';config.write_text('[cursor]\nblink=false\n[shell]\nprogram="/usr/bin/sleep"\nargs=["600"]\n')
 log=(d/'stderr').open('w');binary=os.path.abspath(sys.argv[1])
 p=subprocess.Popen([binary,'--config',str(config),'-e',sys.executable,str(root/'docs/performance/linux-resource-release/measure.py'),'--child',tmp],preexec_fn=debug_parent,stdout=log,stderr=log)
 try:
  deadline=time.monotonic()+30
  while not (d/'child.sock').exists():
   if time.monotonic()>deadline:raise TimeoutError('ready')
   time.sleep(.1)
  conn=socket.socket(socket.AF_UNIX);conn.connect(str(d/'child.sock'));pane=json.loads(conn.makefile('rb').readline())['pane']
  def cli(*args):return json.loads(subprocess.check_output([binary,'panes',*args],text=True))
  def malloc_stats():
   command=['gdb','-q','-batch','-iex','set debuginfod enabled off','-p',str(p.pid),'-ex','call (void)malloc_stats()','-ex','detach']
   status=subprocess.run(command,capture_output=True,text=True,timeout=30)
   time.sleep(.1)
   content=(d/'stderr').read_text()
   import re
   totals=re.findall(r'Total \(incl. mmap\):\nsystem bytes\s*=\s*(\d+)\nin use bytes\s*=\s*(\d+)',content)
   if status.returncode or not totals:raise RuntimeError(status.stdout+status.stderr+content[-2000:])
   system,used=map(int,totals[-1]);return {'system_bytes':system,'in_use_bytes':used}
  time.sleep(3);result={'baseline':m.sample(p.pid),'malloc_baseline':malloc_stats(),'cycles':[],'malloc_cycles':[]}
  for cycle in range(10):
   panes=[cli('create','--beside',pane,'--side','right')['result']['id'] for _ in range(4)];time.sleep(.8)
   for extra in panes:cli('close','--pane',extra)
   time.sleep(.8);result['cycles'].append(m.sample(p.pid));result['malloc_cycles'].append(malloc_stats());print(cycle,result['cycles'][-1],flush=True)
  time.sleep(3);result['before_trim']=m.sample(p.pid);result['malloc_after_cycles']=malloc_stats()
  command=['gdb','-q','-batch','-iex','set debuginfod enabled off','-p',str(p.pid),'-ex','call (int)malloc_trim(0)','-ex','detach']
  gdb=subprocess.run(command,capture_output=True,text=True,timeout=30);result['gdb_status']=gdb.returncode;result['trim_returned_one']='$1 = 1' in gdb.stdout
  time.sleep(2);result['after_trim']=m.sample(p.pid);result['malloc_after_trim']=malloc_stats();print('trim',result['before_trim'],result['after_trim'],flush=True)
  (root/'docs/performance/linux-resource-release/memory-trim.json').write_text(json.dumps(result,indent=2)+'\n')
  cli('close','--pane',pane);p.wait(timeout=15)
 finally:
  if p.poll() is None:p.terminate();p.wait(timeout=10)
