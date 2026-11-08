import React,{useState} from 'react';
import {Copy,Send} from 'lucide-react';
import {Dialog,Field,ErrorMessage} from './ui.jsx';
import {ChannelPreviews} from './deliveryPreview.jsx';

const reasons={
 queued:'Accepted and queued for delivery.',
 route_disabled:'The route is disabled.',
 priority_below_minimum:'The event priority is below the route minimum.',
 no_destinations:'No recipients are linked to this route.',
 no_active_destinations:'All linked recipients are disabled.',
 destination_disabled:'Recipient disabled.',
};

export function RouteTestDialog({route,onClose,onTest,onPreview,onCopy,onViewDeliveries}) {
 const [title,setTitle]=useState('Relayix webhook test'),[message,setMessage]=useState('Test notification from Relayix');
 const [priority,setPriority]=useState('3'),[tagText,setTagText]=useState('');
 const [busy,setBusy]=useState(false),[result,setResult]=useState(null),[error,setError]=useState(''),[notice,setNotice]=useState('');
 const tags=tagText.split(',').map(tag=>tag.trim()).filter(Boolean);
 const valid=(title.trim()||message.trim())&&(priority===''||(Number.isInteger(Number(priority))&&Number(priority)>=1&&Number(priority)<=5))&&tags.length<=32&&tags.every(tag=>tag.length<=64);
 const payload={...(title.trim()?{title}:{}),message,...(priority!==''?{priority:Number(priority)}:{}),...(tags.length?{tags}:{})};
 const update=(setter,value)=>{setter(value);setResult(null);setError('');setNotice('')};
 const send=async e=>{e.preventDefault();if(!valid)return;setBusy(true);setResult(null);setError('');setNotice('');try{setResult(await onTest(payload))}catch(e){setError(e.message);if(e.data?.routing)setResult(e.data)}finally{setBusy(false)}};
 const preview=async()=>{if(!valid)return;setBusy(true);setResult(null);setError('');setNotice('');try{setResult(await onPreview(payload))}catch(e){setError(e.message)}finally{setBusy(false)}};
 const copy=async()=>{setNotice('');try{await onCopy(JSON.stringify(payload,null,2));setNotice('Test JSON copied')}catch(e){setError(e.message)}};
 return <Dialog title="Test webhook" description={route.name} onClose={onClose} busy={busy}><form className="route-test" onSubmit={send}>
  <p className="helptext">Preview checks routing and renders the message for every channel without storing an event or sending a message. Send test webhook sends a real notification through this route. Route priority and recipient status decide delivery; tags are UI labels only.</p>
  <Field label="Test title" help="Optional. Leave empty to send the message without a heading."><input value={title} maxLength={256} onChange={e=>update(setTitle,e.target.value)}/></Field>
  <Field label="Test message" help="Send just a message, or add an optional title and tags. Formatting is automatic for Telegram and Mattermost."><textarea value={message} rows={2} maxLength={10000} onChange={e=>update(setMessage,e.target.value)}/></Field>
  <Field label="Priority (optional)" help={`Minimum for this route: ${route.min_priority}. Webhooks without priority use 3.`}><input type="number" value={priority} min={1} max={5} placeholder="3 (default)" onChange={e=>update(setPriority,e.target.value)}/></Field>
  <Field label="Tags (optional)" help="Optional message labels for filtering delivery history. They do not affect routing."><input value={tagText} placeholder="production, deploy" onChange={e=>update(setTagText,e.target.value)}/></Field>
  <details className="test-json"><summary>Test JSON</summary><pre aria-label="Test webhook JSON">{JSON.stringify(payload,null,2)}</pre></details>
  <div className="test-copy"><button type="button" className="secondary" onClick={copy} disabled={!valid||busy}><Copy size={15}/> Copy test JSON</button><span role="status">{notice}</span></div>
  <ErrorMessage error={error}/>
  {result?.routing&&<section className="routing-result" aria-label="Routing result"><h3>Routing result</h3><p>Route: <strong>{result.routing.route_name}</strong></p><p role="status">{result.preview&&result.would_queue?'Recipients matched; no event stored and no messages sent.':reasons[result.routing.reason]||result.routing.reason}</p><p className="helptext">Priority {result.routing.priority} / minimum {result.routing.min_priority} · Route tag: {result.routing.route_tag||'None'} (UI only)</p><p><strong>{result.preview?`${result.would_queue} would queue · 0 sent`:`${result.queued||0} queued`}</strong> · {result.routing.active_destinations} active / {result.routing.destinations} linked recipients</p>
   {!!result.routing.recipients?.length&&<ul>{result.routing.recipients.map(recipient=><li key={recipient.id}><span>{recipient.name}</span><small>{recipient.would_queue?'Would queue':recipient.queued?'Queued':reasons[recipient.reason]||recipient.reason}</small></li>)}</ul>}
   {!!result.routing.fallback_recipients?.length&&<p className="helptext">Fallback after final failure: {result.routing.fallback_recipients.map(r=>`${r.name}${r.enabled?'':' (disabled)'}`).join(', ')}</p>}
   {result.event_id&&<p className="helptext">Event: <code>{result.event_id}</code></p>}
   {result.preview&&<ChannelPreviews items={result.channel_previews}/>}
   <details><summary>API response</summary><pre>{JSON.stringify(result,null,2)}</pre></details>
   {!!result.queued&&<><p className="helptext">Queued does not mean delivered. Open Deliveries to inspect the send result.</p><button type="button" className="secondary" onClick={onViewDeliveries}>View test deliveries</button></>}
  </section>}
  <div className="dialog-actions"><button type="button" className="secondary" onClick={onClose} disabled={busy}>Close</button><button type="button" className="secondary" onClick={preview} disabled={!valid||busy}>Preview routing and messages</button><button className="primary" disabled={!valid||busy}><Send size={15}/>{busy?'Sending…':'Send test webhook'}</button></div>
 </form></Dialog>;
}
