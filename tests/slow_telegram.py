"""Mounted only by compose_084.yaml: hold subprocess slots without contacting Telegram."""
import json
import os
from pathlib import Path
import sys
import time

request=json.load(sys.stdin)
state=Path('/tmp/relayix-telegram-state')
delivery=request['delivery_id']
with (state/'started.jsonl').open('a') as stream:
    stream.write(json.dumps({'id':delivery,'pid':os.getpid()})+'\n')
deadline=time.monotonic()+50
while not (state/'release').exists() and not (state/(delivery+'.release')).exists():
    (state/(delivery+'.heartbeat')).write_text(str(time.monotonic()))
    if time.monotonic()>deadline:raise SystemExit(1)
    time.sleep(.1)
(state/(delivery+'.finished')).touch()
print(json.dumps({'ok':True}))
