"""Authenticated local HTTP forward/CONNECT proxy; upstreams restricted to test mocks."""
import base64
from runtime import credentials
import http.client
import select
import socket
import threading
from http.server import BaseHTTPRequestHandler,ThreadingHTTPServer
from urllib.parse import urlsplit

class Proxy(BaseHTTPRequestHandler):
    protocol_version='HTTP/1.1'
    def log_message(self,*args):pass
    def allowed(self):
        if self.headers.get('Proxy-Authorization')=='Basic '+base64.b64encode((credentials['proxy_username']+':'+credentials['proxy_password']).encode()).decode():return True
        self.send_response(407);self.send_header('Content-Length','0');self.end_headers();return False
    def do_CONNECT(self):
        if not self.allowed():return
        if self.path not in ('mock:1025',socket.gethostbyname('mock')+':1025'):
            self.send_error(403);return
        with socket.create_connection(('mock',1025),timeout=5) as remote:
            self.send_response(200);self.end_headers()
            sockets=[self.connection,remote]
            while True:
                ready,_,_=select.select(sockets,[],[],20)
                if not ready:return
                for source in ready:
                    data=source.recv(65536)
                    if not data:return
                    (remote if source is self.connection else self.connection).sendall(data)
    def forward(self):
        if not self.allowed():return
        url=urlsplit(self.path)
        if url.hostname!='mock' or url.port!=8000:self.send_error(403);return
        body=self.rfile.read(int(self.headers.get('Content-Length',0)))
        headers={k:v for k,v in self.headers.items() if k.lower() not in ('proxy-authorization','proxy-connection','connection','host')}
        headers['X-Relayix-Test-Proxy']='used'
        upstream=http.client.HTTPConnection('mock',8000,timeout=5)
        upstream.request(self.command,url.path+('?' + url.query if url.query else ''),body,headers)
        response=upstream.getresponse();data=response.read()
        self.send_response(response.status);self.send_header('Content-Type','application/json');self.send_header('Content-Length',str(len(data)));self.end_headers();self.wfile.write(data)
        upstream.close()
    do_GET=forward
    do_POST=forward
    do_PUT=forward

ThreadingHTTPServer(('0.0.0.0',3128),Proxy).serve_forever()
