#!/usr/bin/env python3
"""Re-records app/e2e/fixtures/rates.json and sims.json from the real core: builds a temp
home with the synthetic sim files, runs `quadcam-cli gear rates|sims` (debug build) on the
synthetic three-profile dump. Run it when a rates or sims shape changes."""
import struct, os, json, subprocess, tempfile, shutil
root = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
src = root + '/src-tauri'
fx = src + '/tests/fixtures/sims/'
h = tempfile.mkdtemp(dir=tempfile.gettempdir())
def put(rel, data):
    p = os.path.join(h, 'Library/Application Support', rel)
    os.makedirs(os.path.dirname(p), exist_ok=True)
    open(p, 'wb').write(data)
put('LuGus Studios/Liftoff/Saves/Player/UserData.xml', open(fx + 'liftoff.UserData.xml', 'rb').read())
put('Steam/steamapps/common/Liftoff Micro Drones/Liftoff Micro Drones.app/Contents/Saves/Player/UserData.xml', open(fx + 'micro.UserData.xml', 'rb').read())
put('Godot/app_userdata/The Zone/settings.cfg', open(fx + 'zone.settings.cfg', 'rb').read())
def gv(fl):
    b = b'GVAS' + bytes([3, 0, 0, 0, 10, 2, 0, 0])
    for s in ('Rates', 'ArrayProperty', 'FloatProperty'):
        b += struct.pack('<i', len(s) + 1) + s.encode() + b'\0'
    return b + b'\0' + struct.pack('<i', len(fl)) + b''.join(struct.pack('<f', f) for f in fl) + b'\x05\0\0\0None\0'
put('Uncrashed/abc123/rates/FREE.sav', gv([0.72, 1.27, 0.40, 0.72, 1.27, 0.40, 0.75, 1.00, 0.0, 0.0, 0.30, 0.50]))
put('Uncrashed/abc123/rates/EMPTY.sav', gv([]))
env = dict(os.environ, HOME=h, QUADCAM_PHOTOS='dry-run', QUADCAM_SIMS_RUNNING='Liftoff', CARGO_MANIFEST_DIR=src)
q = src + '/tests/fixtures/rates/three-profiles.dump_all.txt'
cli = src + '/target/debug/quadcam-cli'
def run(*a):
    r = subprocess.run([cli, '--json', 'gear', *a], env=env, capture_output=True, text=True)
    return json.loads(r.stdout)['result']
json.dump(run('rates', q), open(root + '/app/e2e/fixtures/rates.json', 'w'), indent=1)
out = {'none': run('sims')}
for i in range(3):
    out[f'p{i}'] = run('sims', q, '--profile', str(i))
json.dump(out, open(root + '/app/e2e/fixtures/sims.json', 'w'), indent=1)
print({k: [(s['id'], s['in_sync']) for s in v] for k, v in out.items()})
shutil.rmtree(h)
