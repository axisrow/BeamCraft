"""Original synthetic fixtures for the four visual findings; MPL-2.0.
Run: python3 reproduce-fixtures.py /absolute/path/to/beamcraft /new/output/directory
Creates only new fixture stores; refuses existing output to avoid altering user data.
"""
import json
import pathlib
import subprocess
import sys

cli=pathlib.Path(sys.argv[1]).resolve(strict=True)
root=pathlib.Path(sys.argv[2]).absolute()
root.mkdir(exist_ok=False)

def run(store,command,*args):
    path=root/store
    path.mkdir(exist_ok=True)
    result=subprocess.run([str(cli),command,'--store',str(path),*args],capture_output=True,text=True,check=True)
    return json.loads(result.stdout)

run('volume','define-profile','--width','200','--height','300','--name','Demo')
run('circle','create-plate','--width','1000','--length','1000','--thickness','20',
    '--hole-round','500,500,500','--hole-segments','128')
for name in ['Roof beams','Floor beams']:
    run('names','define-profile','--width','200','--height','300','--name',name)
for name in ['Structural steel','Test material']:
    run('names','define-material','--density','7850','--name',name)
run('names','create-plate','--width','500','--length','500','--thickness','20')
print('Fixture stores:',root)
print('Open each store in beamcraft-ui and follow REPORT.md. volume starts empty intentionally.')
