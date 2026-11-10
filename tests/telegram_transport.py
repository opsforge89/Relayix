"""MTProto/WEB wiring and error redaction checks; no Telegram connections."""
import asyncio
import sys,secrets,uuid
sys.path.insert(0,"/app/transport")
import importlib.util
from pathlib import Path
from telethon import connection, types
from telethon_webproxy import ConnectionWebProxy

spec=importlib.util.spec_from_file_location('relayix_telegram',Path('/app/transport/telegram_send.py'))
module=importlib.util.module_from_spec(spec);spec.loader.exec_module(module)
cfg={'proxy_type':'mtproto','proxy_host':'proxy.example.org','proxy_port':'443','proxy_secret':'dd'+secrets.token_hex(16),
     'api_id':'12345','api_hash':secrets.token_hex(16),'bot_token':secrets.token_urlsafe(32),'chat_id':'@fixture','message_thread_id':'42'}
assert module.options(cfg)['connection'] is connection.ConnectionTcpMTProxyRandomizedIntermediate
for prefix in ('01','dd','ee'):
    plain=prefix+secrets.token_hex(15)
    selected=module.options({**cfg,'proxy_secret':plain})['connection']
    assert selected is module.ConnectionTcpMTProxyPlain
    assert issubclass(selected,connection.ConnectionTcpMTProxyAbridged)
    assert selected.normalize_secret(plain)==bytes.fromhex(plain)
    assert module.options({**cfg,'proxy_type':'webproxy','proxy_secret':plain,'webproxy_address':'proxy.example.org'})['connection'] is ConnectionWebProxy
assert module.options({**cfg,'proxy_type':'webproxy','webproxy_address':'https://PROXY.example.org'})['connection'] is ConnectionWebProxy
assert module.destination({**cfg,'chat_id':'-1000000000123','chat_access_hash':'456'})==types.InputPeerChannel(123,0)
assert module.destination({**cfg,'chat_id':'79419177'})==types.InputPeerUser(79419177,0)
assert module.destination({**cfg,'chat_id':'-1000000000123'})==types.InputPeerChannel(123,0)
assert module.destination({**cfg,'chat_id':'-123'})==types.InputPeerChat(123)
seen=[]
class FakeClient:
    def __init__(self,*args,**kwargs):seen.append(kwargs)
    async def start(self,**kwargs):assert kwargs['bot_token']==cfg['bot_token']
    async def get_input_entity(self,entity):return types.InputPeerSelf()
    async def __call__(self,request):seen.append(request)
    async def disconnect(self):seen.append('disconnected')
data={'config':cfg,'text':'Local transport check','delivery_id':str(uuid.uuid4())}
asyncio.run(module.deliver(data,FakeClient));request=seen[-2]
assert request.message=='Local transport check' and request.reply_to.reply_to_msg_id==42
assert seen[-1]=='disconnected'
ee='ee'+secrets.token_hex(16)+'front.example.org'.encode().hex()
from faketls import ConnectionTcpMTProxyFakeTLS
assert module.options({**cfg,'proxy_secret':ee})['connection'] is ConnectionTcpMTProxyFakeTLS
try:module.options({**cfg,'proxy_secret':ee,'proxy_type':'webproxy','webproxy_address':'proxy.example.org'});raise AssertionError('WEB accepted ee')
except ValueError:pass
for mode in ['mtproto','webproxy']:
    bad={**cfg,'proxy_type':mode,'proxy_secret':'ee'+secrets.token_hex(16),'webproxy_address':'proxy.example.org'}
    try:module.options(bad);raise AssertionError('accepted FakeTLS secret')
    except ValueError:pass
assert module.error_code(ConnectionError('password=do-not-print'))=='proxy'
print('PASS: real MTProto/WEB connector selection, in-memory sessions, destination/topic mapping, bounded delivery and safe error classification')

class DiagnosticClient(FakeClient):
    async def connect(self):seen.append('connected')
    async def sign_in(self,**kwargs):
        assert kwargs['bot_token']==cfg['bot_token']
        seen.append('authenticated')
    async def get_entity(self,entity):
        seen.append('resolved')
        return types.InputPeerSelf()
    async def __call__(self,request):
        raise AssertionError('Diagnostics must not send messages')
seen.clear()
steps=asyncio.run(module.diagnose(data,DiagnosticClient))
assert [s['name'] for s in steps]==['transport','credentials','destination']
assert [s['status'] for s in steps]==['passed','passed','passed']
assert seen[-1]=='disconnected'
for stage in ('connect','sign_in','get_entity'):
    async def failure(self,*args,**kwargs):
        raise ConnectionError('private-credential-must-not-leak')
    failing=type('FailingClient',(DiagnosticClient,),{stage:failure})
    steps=asyncio.run(module.diagnose(data,failing))
    assert steps[-1]['status']=='failed'
    assert steps[-1]['name']=={'connect':'transport','sign_in':'credentials','get_entity':'destination'}[stage]
    assert 'private-credential' not in str(steps)
print('PASS: Telegram diagnostics report individual stages without sending and redact provider exceptions')

# HTML is converted to MTProto entities with UTF-16 offsets, including emoji.
seen.clear()
asyncio.run(module.deliver({**data,'text':'✅ <b>Passed</b> &lt;b&gt;literal&lt;/b&gt;','parse_mode':'HTML'},FakeClient))
request=seen[-2]
assert request.message=='✅ Passed <b>literal</b>'
assert len(request.entities)==1 and isinstance(request.entities[0],types.MessageEntityBold)
assert request.entities[0].offset==2 and request.entities[0].length==6
seen.clear()
asyncio.run(module.deliver({**data,'text':'🚀 <b>OK</b>','parse_mode':'HTML'},FakeClient))
assert seen[-2].entities[0].offset==3 and seen[-2].entities[0].length==2
print('PASS: Telegram HTML becomes MTProto entities with correct emoji offsets and literal escaped tags')

seen.clear()
asyncio.run(module.deliver({**data,'text':'  <b>OK</b>\n\n','parse_mode':'HTML'},FakeClient))
assert seen[-2].message=='  OK\n\n' and seen[-2].entities[0].offset==2
seen.clear()
asyncio.run(module.deliver({**data,'text':'Tasks:\n<a href="https://tickets.example.org/project/Task/TASK-123">TASK-123</a>','parse_mode':'HTML'},FakeClient))
assert seen[-2].message=='Tasks:\nTASK-123'
assert len(seen[-2].entities)==1 and isinstance(seen[-2].entities[0],types.MessageEntityTextUrl)
assert seen[-2].entities[0].url=='https://tickets.example.org/project/Task/TASK-123'

# WEB keeps its TLS hostname but never re-resolves a checked address.
if hasattr(module,'pin_proxy_dns'):
    import socket
    original=socket.getaddrinfo
    try:
        module.pin_proxy_dns({'proxy_type':'webproxy','outbound_proxy_host':'proxy.example.org','outbound_proxy_ip':'203.0.113.7'})
        assert socket.getaddrinfo('proxy.example.org',443)[0][4][0]=='203.0.113.7'
        try:socket.getaddrinfo('localhost',443);raise AssertionError('Unapproved hostname was resolved')
        except OSError:pass
    finally:socket.getaddrinfo=original
    print('PASS: WEB proxy DNS pinning preserves hostname and blocks alternate resolutions')

# Files stay in memory and preserve topic and deterministic document message IDs.
import base64
class FileClient(FakeClient):
    async def upload_file(self,content,*,file_name):
        assert content.read()==b'PK\x00\xffxlsx-report'
        assert file_name=='report.xlsx' and content.name==file_name
        seen.append('uploaded')
        return types.InputFile(id=1,parts=1,name=file_name,md5_checksum='')
file_data={**data,'files':[{'name':'report.xlsx','content_type':'application/vnd.openxmlformats-officedocument.spreadsheetml.sheet','data':base64.b64encode(b'PK\x00\xffxlsx-report').decode()}]}
seen.clear();asyncio.run(module.deliver(file_data,FileClient))
request=seen[-2];first_id=request.random_id
assert isinstance(request, module.functions.messages.SendMediaRequest)
assert request.reply_to.reply_to_msg_id==42 and request.media.attributes[0].file_name=='report.xlsx'
seen.clear();asyncio.run(module.deliver({**file_data,'text':''},FileClient))
assert seen[-2].random_id==first_id
assert not any(isinstance(item,module.functions.messages.SendMessageRequest) for item in seen)
print('PASS: MTProto/WEB upload document bytes in memory, preserve filenames/topics and reuse stable per-delivery media IDs')
