"""Independently compare every pressed audio sector with the source CUEs."""
from pathlib import Path
import hashlib,json,re,sys
sys.path.insert(0, str(Path(__file__).resolve().parent))
from check_program_headless import parse_entries,read_user_sectors
S=2352

def cue(path):
    path=Path(path); tracks=[]; image=None
    for line in path.read_text().splitlines():
        m=re.match(r'\s*FILE "([^"]+)" BINARY',line)
        if m:
            if image is not None: raise ValueError('multiple FILEs unsupported')
            image=path.parent/m[1]
        m=re.match(r'\s*TRACK (\d+) (\S+)',line)
        if m: tracks.append({'number':int(m[1]),'type':m[2]})
        m=re.match(r'\s*INDEX (00|01) (\d+):(\d+):(\d+)',line)
        if m: tracks[-1][m[1]]=int(m[2])*4500+int(m[3])*75+int(m[4])
    assert image.stat().st_size%S==0
    for i,t in enumerate(tracks):
        assert t['number']==i+1 and '01' in t
        t['start']=t.get('00',t['01']);t['end']=tracks[i+1].get('00',tracks[i+1]['01']) if i+1<len(tracks) else image.stat().st_size//S
        assert t['start']<=t['01']<t['end']
    return image,tracks

def digest(path,start,end):
    h=hashlib.sha256()
    with path.open('rb') as f:
        f.seek(start*S);remaining=(end-start)*S
        while remaining:
            b=f.read(min(1048576,remaining));assert b;h.update(b);remaining-=len(b)
    return h.hexdigest()

p=Path.cwd(); combined,tracks=cue(sys.argv[1]); entries={e.name:e.cdda_track_base for e in parse_entries(read_user_sectors(combined,22,4))}; report=[]
menus=['knuckle-dust','rusted-hammer','chainsaw-heart','night-crawler']
for i,name in enumerate(menus,2):
    t=tracks[i-1];raw=(p/'audio'/f'{name}.cdda').read_bytes(); expected=raw+bytes((-len(raw))%S)
    assert hashlib.sha256(expected).hexdigest()==digest(combined,t['01'],t['end']),name
    report.append({'owner':'LAUNCHER','track':i,'sha256':hashlib.sha256(expected).hexdigest()})
inputs=[('CORTEX IGNITION',p/'build/cortex-current-04b/baked/cortex_ignition_tech_demo_0_4b.cue')]
if 'HALF-LIFE' in entries: inputs += [('HALF-LIFE',p/'games/hl-psx/dist/hl-psx.cue')]
inputs += [('VOXIDE',p/'games/voxide/dist/voxide.cue'),('NITROXIDE',p/'build/nitroxide/NitroXide/NitroXide.cue'),('GH-PSX',p/'games/gh-psx/dist/gh-psx.cue'),('PSOXIDE ARCADE',p/'games/psoxide-arcade/dist/psoxide-arcade.cue'),('HARDWARE TESTS',p/'games/PSoXide-editor/build/examples/mipsel-sony-psx/release/hardware-tests.cue'),('QUAKE SHAREWARE',p.parent/'quake-psx/dist/quake-psx.cue')]
base=4
for name,path in inputs:
    source,original=cue(path)
    # Data-only guests may appear earlier in the packer's image order; their
    # unused CD audio base does not describe a track owned by this guest.
    if len(original)>1 and name!='GH-PSX': assert entries[name]==base,(name,entries[name],base)
    for st in original[1:]:
        t=tracks[base+st['number']-1]
        assert t['type']=='AUDIO' and st['type']=='AUDIO'
        assert t['01']-t['start']==st['01']-st['start'],name
        assert t['end']-t['start']==st['end']-st['start'],name
        h=digest(source,st['start'],st['end']);assert h==digest(combined,t['start'],t['end']),name
        report.append({'owner':name,'source_track':st['number'],'track':t['number'],'sectors':t['end']-t['01'],'sha256_with_pregap':h})
    base+=len(original)-1
assert entries['GH-PSX']==entries['PSOXIDE ARCADE']
assert entries['CELESTE COLLECTION']==entries['PSXCEL']==0
assert len(tracks)==base+1
assert len(report)==len(tracks)-1
print(json.dumps({'cue':sys.argv[1],'audio_tracks':len(report),'track_bases':entries,'tracks':report,'result':'PASS'},indent=2))
