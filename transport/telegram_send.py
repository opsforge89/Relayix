# SPDX-License-Identifier: Apache-2.0
"""One bounded MTProto delivery. Secrets arrive on stdin, never argv or disk."""
import asyncio
import json
import logging
import sys
import socket
import ipaddress
import uuid
from urllib.parse import urlsplit
from telethon import TelegramClient, connection, functions, types, errors
from telethon.sessions import MemorySession
from telethon.extensions.html import HTMLToTelegramParser
from telethon.helpers import add_surrogate, del_surrogate
from telethon_webproxy import ConnectionWebProxy
from faketls import ConnectionTcpMTProxyFakeTLS, parse_secret

logging.disable(logging.CRITICAL)


class ConnectionTcpMTProxyPlain(connection.ConnectionTcpMTProxyAbridged):
    @staticmethod
    def normalize_secret(secret):
        # A 16-byte raw key can itself begin with dd/ee; those bytes are not markers.
        if len(secret) != 32:
            raise ValueError('Plain MTProxy secrets must contain 16 bytes')
        return bytes.fromhex(secret)


def options(cfg):
    secret = cfg['proxy_secret']
    if secret.startswith('ee') and len(secret) != 32:
        parse_secret(secret)
        if cfg['proxy_type'] != 'mtproto':
            raise ValueError('WEB-proxy does not support FakeTLS')
        return {'connection': ConnectionTcpMTProxyFakeTLS, 'proxy': (cfg['proxy_host'].strip('[]'), int(cfg['proxy_port']), secret)}
    if len(secret) not in (32, 34) or len(secret) == 34 and not secret.startswith('dd'):
        raise ValueError('Unsupported proxy secret')
    bytes.fromhex(secret)
    if cfg['proxy_type'] == 'mtproto':
        mode = connection.ConnectionTcpMTProxyRandomizedIntermediate if len(secret) == 34 else ConnectionTcpMTProxyPlain
        return {'connection': mode, 'proxy': (cfg['proxy_host'].strip('[]'), int(cfg['proxy_port']), secret)}
    if cfg['proxy_type'] == 'webproxy':
        url = urlsplit('https://' + cfg['webproxy_address'].removeprefix('https://'))
        if not url.hostname or url.path not in ('', '/') or url.port or url.query or url.fragment or url.username:
            raise ValueError('WEB-proxy requires a root HTTPS hostname')
        return {'connection': ConnectionWebProxy, 'proxy': (url.hostname.encode('idna').decode().lower(), secret, {'mode': 'websocket'})}
    raise ValueError('MTProto transport requires an explicit proxy')


def destination(cfg):
    target = str(cfg['chat_id']).strip()
    if target.startswith('@'):
        return target
    chat = int(target)
    # Telegram bots use a zero access hash when no session-local full hash exists.
    # https://core.telegram.org/api/peers#access-hash
    if chat < -1000000000000:
        return types.InputPeerChannel(-chat - 1000000000000, 0)
    if chat < 0:
        return types.InputPeerChat(-chat)
    return types.InputPeerUser(chat, 0)


def formatted_text(text):
    # Use Telethon's UTF-16-aware parser without stripping the supplied message.
    parser = HTMLToTelegramParser()
    parser.feed(add_surrogate(text))
    parser.close()
    parser.entities.reverse()
    parser.entities.sort(key=lambda entity: entity.offset)
    return del_surrogate(parser.text), parser.entities


async def deliver(data, factory=TelegramClient):
    cfg = data['config']
    client = factory(MemorySession(), int(cfg['api_id']), cfg['api_hash'],
                     **options(cfg), timeout=10, connection_retries=1, request_retries=0,
                     auto_reconnect=False, receive_updates=False, flood_sleep_threshold=0)
    try:
        await client.start(bot_token=cfg['bot_token'])
        peer = await client.get_input_entity(destination(cfg))
        topic = cfg.get('message_thread_id')
        reply = types.InputReplyToMessage(reply_to_msg_id=int(topic)) if topic else None
        random_id = int.from_bytes(uuid.UUID(data['delivery_id']).bytes[:8], 'big', signed=True)
        text, entities = formatted_text(data['text']) if data.get('parse_mode') == 'HTML' else (data['text'], [])
        await client(functions.messages.SendMessageRequest(peer=peer, message=text, entities=entities,
                     random_id=random_id, reply_to=reply, no_webpage=True))
    finally:
        await client.disconnect()


async def diagnose(data, factory=TelegramClient):
    cfg = data['config']
    steps = []
    client = factory(MemorySession(), int(cfg['api_id']), cfg['api_hash'],
                     **options(cfg), timeout=10, connection_retries=1, request_retries=0,
                     auto_reconnect=False, receive_updates=False, flood_sleep_threshold=0)
    stage = 'transport'
    try:
        await client.connect()
        steps.append({'name': stage, 'status': 'passed', 'detail': 'Proxy transport and Telegram handshake succeeded.'})
        stage = 'credentials'
        await client.sign_in(bot_token=cfg['bot_token'])
        steps.append({'name': stage, 'status': 'passed', 'detail': 'Telegram accepted the bot and application credentials.'})
        stage = 'destination'
        await client.get_entity(destination(cfg))
        steps.append({'name': stage, 'status': 'passed', 'detail': 'Telegram resolved the destination.'})
    except Exception as exc:
        steps.append({'name': stage, 'status': 'failed', 'detail': {
            'credentials': 'Telegram rejected credentials.', 'destination': 'Chat not accessible; check bot membership and Chat ID.',
            'flood': 'Telegram rate limit; wait before retrying.', 'proxy': 'Proxy transport or Telegram handshake failed.'
        }.get(error_code(exc), 'Telegram check failed; check connectivity and permissions.')})
    finally:
        await client.disconnect()
    return steps


def error_code(exc):
    if isinstance(exc, errors.FloodWaitError):
        return 'flood'
    if isinstance(exc, (errors.ApiIdInvalidError, errors.AccessTokenInvalidError, errors.AccessTokenExpiredError)):
        return 'credentials'
    if isinstance(exc, (ValueError, KeyError, errors.PeerIdInvalidError, errors.UsernameNotOccupiedError)):
        return 'destination'
    if isinstance(exc, (OSError, ConnectionError, asyncio.TimeoutError)):
        return 'proxy'
    return 'delivery'


def pin_proxy_dns(cfg):
    """Preserve the WEB TLS hostname while using the address checked by Rust."""
    if cfg.get('proxy_type')!='webproxy':return
    host=cfg['outbound_proxy_host'].encode('idna').decode().lower()
    address=str(ipaddress.ip_address(cfg['outbound_proxy_ip']))
    original=socket.getaddrinfo
    def pinned(name,port,*args,**kwargs):
        if isinstance(name,bytes):name=name.decode('ascii')
        if name.lower().rstrip('.')!=host:raise OSError('Unapproved proxy DNS lookup')
        return original(address,port,*args,**kwargs)
    socket.getaddrinfo=pinned


def main():
    try:
        raw = sys.stdin.buffer.read(128 * 1024 + 1)
        if len(raw) > 128 * 1024:
            raise ValueError('Request too large')
        data = json.loads(raw)
        pin_proxy_dns(data['config'])
        if data.get('action') == 'diagnose':
            steps = asyncio.run(asyncio.wait_for(diagnose(data), timeout=45))
            print(json.dumps({'ok': True, 'steps': steps}))
        else:
            asyncio.run(asyncio.wait_for(deliver(data), timeout=45))
            print(json.dumps({'ok': True}))
    except Exception as exc:
        # Never echo Telegram/proxy exception strings, tokens, URLs or configuration.
        print(json.dumps({'ok': False, 'code': error_code(exc), **({'retry_after': max(0, int(exc.seconds))} if isinstance(exc, errors.FloodWaitError) else {})}))
        raise SystemExit(1)


if __name__ == '__main__':
    main()
