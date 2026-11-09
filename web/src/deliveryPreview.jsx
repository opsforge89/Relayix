import React,{useState} from 'react';
import {MessageBody} from './message.jsx';
import {integrations,ProviderIcon} from './integrations.jsx';
import {Field,ErrorMessage} from './ui.jsx';

function TelegramText({text}) {
 const nodes=new DOMParser().parseFromString(text,'text/html').body.childNodes;
 const render=node=>{
  if(node.nodeType===3)return node.textContent;
  const children=Array.from(node.childNodes).map((child,index)=><React.Fragment key={index}>{render(child)}</React.Fragment>);
  if(node.nodeName==='B')return <strong>{children}</strong>;
  if(node.nodeName==='A'){
   try{const url=new URL(node.getAttribute('href'));if(['http:','https:'].includes(url.protocol)&&!url.username&&!url.password)return <a href={url.href} target="_blank" rel="noopener noreferrer">{children}</a>}catch{}
  }
  return <>{children}</>;
 };
 return <div className="message-view">{Array.from(nodes).map((node,index)=><React.Fragment key={index}>{render(node)}</React.Fragment>)}</div>;
}
export function DeliveryPreview({value}) {
 if(value.restricted||value.error)return <p className="helptext">{value.reason||value.error}</p>;
 return <div className="delivery-preview"><p role="status" className={value.truncated?'delivery-error':'helptext'}>{value.truncated?'This delivery will be shortened. The marker shows where content ends.':'This delivery fits the Relayix limits.'}</p>{value.limits?.map(limit=><p className="helptext" key={limit.field}>{limit.field}: <strong>{limit.used.toLocaleString()} / {limit.maximum.toLocaleString()} {limit.unit}</strong></p>)}{!value.limits?.length&&<p className="helptext">Destination server limits may also apply.</p>}{value.format==='html'?<TelegramText text={value.text}/>:value.format==='markdown'?<MessageBody text={value.text}/>:<pre aria-label="Outgoing message">{value.text}</pre>}<details><summary>Exact outgoing text</summary><pre>{value.text}</pre></details></div>;
}
export function ChannelPreviews({items=[]}) {
 if(!items.length)return null;
 return <section aria-label="Message previews"><h3>Message previews</h3><p className="helptext">Rendered with the route template and each channel's delivery limits. Preview sends no requests to destination services.</p>{items.map(channel=><section className="channel-message-preview" key={channel.id}><h4><ProviderIcon kind={channel.kind}/> {channel.name} · {integrations[channel.kind]?.label||channel.kind}{channel.is_fallback?' · Fallback':''}{!channel.enabled?' · Disabled':''}{channel.paused?' · Paused':''}</h4><DeliveryPreview value={channel.presentation}/></section>)}</section>;
}
export function DraftMessagePreview({draft,api}) {
 const [title,setTitle]=useState('Preview notification'),[message,setMessage]=useState('Your message appears here.'),[result,setResult]=useState(null),[error,setError]=useState(''),[busy,setBusy]=useState(false);
 const [snapshot,setSnapshot]=useState('');
 const key=JSON.stringify({kind:draft.kind,config:draft.config,title,message});
 const preview=async()=>{setBusy(true);setError('');const current=key;try{const value=await api('/api/channels/preview',{method:'POST',body:JSON.stringify({kind:draft.kind,config:draft.config,title,message})});setResult(value);setSnapshot(current)}catch(e){setError(e.message)}finally{setBusy(false)}};
 return <details className="draft-message-preview"><summary>Preview a message</summary><p className="helptext">Check formatting and length before saving. Nothing is sent or stored.</p><Field label="Preview title"><input value={title} onChange={e=>setTitle(e.target.value)} maxLength={256}/></Field><Field label="Preview message"><textarea value={message} rows={4} onChange={e=>setMessage(e.target.value)} maxLength={10000}/></Field><button type="button" className="secondary" onClick={preview} disabled={busy}>{busy?'Rendering…':'Preview message'}</button><ErrorMessage error={error}/>{result&&snapshot===key&&<DeliveryPreview value={result}/>}</details>;
}
