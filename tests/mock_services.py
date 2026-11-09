"""Local HTTP providers and SMTP relay; never contacts real services."""
import json
import time
from runtime import credentials
import socketserver
import threading
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

received = []
lock = threading.Lock()


recovered=set()


class HTTP(BaseHTTPRequestHandler):
    def log_message(self, *args):
        pass

    def reply(self, status, data, headers=None):
        self.send_response(status)
        self.send_header('Content-Type', 'application/json')
        for key,value in (headers or {}).items():self.send_header(key,value)
        self.end_headers()
        self.wfile.write(json.dumps(data).encode())

    def do_GET(self):
        if self.path.startswith('/control088/'):
            with lock:recovered.add(self.path.rsplit('/',1)[-1])
            return self.reply(200, {'ok':True})
        if self.path.startswith('/api/v4/'):
            with lock:
                received.append({'path': self.path, 'method': 'GET', 'authorization': self.headers.get('Authorization'), 'body': ''})
            if self.headers.get('Authorization') != 'Bearer ' + credentials['mattermost_token']:
                return self.reply(401, {'message': 'Invalid token'})
            if self.path.startswith('/api/v4/channels/'):
                return self.reply(200, {'id': self.path.rsplit('/',1)[1]})
            if self.path == '/api/v4/users/me':
                return self.reply(200, {'id': 's' * 26})
            if self.path == '/api/v4/users/username/ops-bot':
                return self.reply(200, {'id': 'r' * 26, 'username': 'ops-bot'})
            return self.reply(404, {'message': 'Not found'})
        if self.path in ('/v1/health','/health'):
            return self.reply(200, {'healthy':True,'health':'green'})
        if self.path.endswith('/account/whoami'):
            return self.reply(200, {'user_id':'@example:test'})
        if self.path.endswith('/joined_rooms'):
            return self.reply(200, {'joined_rooms':['!test:local']})
        if self.path == '/discord-diagnostic':
            return self.reply(200, {'id':'example-webhook'})
        if self.path == '/received':
            with lock:
                self.reply(200, received)
        elif self.path.endswith('/state/m.room.encryption'):
            self.reply(200 if 'encrypted' in self.path else 404, {'algorithm': 'm.megolm.v1.aes-sha2'})
        else:
            self.reply(404, {})

    def do_POST(self):
        if self.path=='/slow':time.sleep(5)
        body = self.rfile.read(int(self.headers.get('Content-Length', 0))).decode()
        with lock:
            received.append({'path': self.path, 'method': self.command, 'body': body,
                             'authorization': self.headers.get('Authorization'), 'test_proxy': self.headers.get('X-Relayix-Test-Proxy')})
        if self.path.startswith('/resilience088/'):
            case=self.path.rsplit('/',1)[-1]
            if case=='huge_rate':return self.reply(429,{'parameters':{'retry_after':1000000}}, {'Retry-After':'1000000'})
            if case=='message' and json.loads(body).get('message')=='bad':return self.reply(413,{'error':'message rejected'})
            if case=='rate':return self.reply(429,{'parameters':{'retry_after':120}}, {'Retry-After':'20'})
            if case=='auth':return self.reply(401,{'error':'credential echo must never enter delivery history'})
            if case=='flaky':
                identity=self.path.split('/')[2]
                with lock:healthy=identity in recovered
                if not healthy:return self.reply(503,{'error':'temporary failure'})
                time.sleep(2)
        if self.path.startswith('/limit-ntfy087-') and len(body.encode())>4096:
            return self.reply(400, {'error':'body exceeds provider byte limit'})
        if self.path.startswith('/limit-google087-') and len(body.encode())>32000:
            return self.reply(400, {'error':'JSON body exceeds provider byte limit'})
        if self.path.startswith('/limit-discord087-') and len(json.loads(body).get('content','').encode('utf-16-le'))//2>2000:
            return self.reply(400, {'error':'content exceeds provider limit'})
        if self.path.startswith('/api/v4/'):
            if self.headers.get('Authorization') != 'Bearer ' + credentials['mattermost_token']:
                return self.reply(401, {'message': 'Invalid token'})
            data = json.loads(body)
            if self.path == '/api/v4/channels/direct':
                if data != ['s' * 26, 'r' * 26]:
                    return self.reply(400, {'message': 'Invalid users'})
                return self.reply(201, {'id': 'd' * 26})
            if self.path == '/api/v4/posts':
                if data.get('channel_id') == 'forbidden':
                    return self.reply(403, {'message': 'No permission'})
                if data.get('channel_id') == 'malformed':
                    return self.reply(200, {'unexpected': True})
                return self.reply(201, {'id': 'p' * 26, 'channel_id': data['channel_id'], 'message': data['message']})
            return self.reply(404, {'message': 'Unknown API path'})
        self.reply(503 if self.path == '/fail' else 200, {'event_id': '$test', 'ok': True})

    do_PUT = do_POST


class SMTP(socketserver.StreamRequestHandler):
    def handle(self):
        def reply(line):
            self.wfile.write((line + '\r\n').encode())
            self.wfile.flush()
        reply('220 mock ESMTP')
        while line := self.rfile.readline():
            cmd = line.decode().strip()
            if cmd.upper().startswith(('EHLO', 'HELO')):
                reply('250-mock\r\n250 AUTH PLAIN')
            elif cmd.upper().startswith('AUTH PLAIN'):
                reply('235 Authentication successful')
            elif cmd.upper().startswith(('MAIL FROM:', 'RCPT TO:', 'RSET', 'NOOP')):
                reply('250 OK')
            elif cmd.upper() == 'DATA':
                reply('354 End with a dot')
                lines = []
                while (line := self.rfile.readline()) not in (b'.\r\n', b''):
                    lines.append(line)
                with lock:
                    received.append({'method': 'SMTP', 'body': b''.join(lines).decode()})
                reply('250 Accepted')
            elif cmd.upper() == 'QUIT':
                reply('221 Bye')
                return
            else:
                reply('502 Unsupported')


smtp = socketserver.ThreadingTCPServer(('0.0.0.0', 1025), SMTP)
threading.Thread(target=smtp.serve_forever, daemon=True).start()
ThreadingHTTPServer(('0.0.0.0', 8000), HTTP).serve_forever()
