import React from 'react';
import {CheckCircle2,XCircle,RefreshCw,Clock3,LoaderCircle} from 'lucide-react';
import {ProviderIcon,integrations} from './integrations.jsx';

export function deliveryState(recipient) {
 const key=recipient.status==='queued'&&recipient.error?'retrying':recipient.status;
 return {key,label:({sent:'Sent',failed:'Failed',retrying:'Retrying',queued:'Queued',processing:'Sending',cancelled:'Cancelled',expired:'Expired'})[key]||key,Icon:({sent:CheckCircle2,failed:XCircle,retrying:RefreshCw,queued:Clock3,processing:LoaderCircle,cancelled:XCircle,expired:Clock3})[key]||Clock3};
}
export function RecipientChip({recipient,onSelect,filterActive=false}) {
 const {key,label,Icon}=deliveryState(recipient),service=integrations[recipient.channel_kind]?.label||recipient.channel_kind;
 const context=filterActive&&!recipient.matches_filter;
 return <button type="button" data-delivery-id={recipient.id} className={`recipient-chip recipient-${key}${context?' recipient-context':''}${filterActive&&recipient.matches_filter?' recipient-match':''}`} aria-label={`View delivery ${recipient.channel_name} (${service}): ${label}`} title={`${service} · ${recipient.channel_name} · ${label}${recipient.is_fallback?' · Fallback':''}${context?' · Other recipient':''}`} onClick={()=>onSelect(recipient)}><ProviderIcon kind={recipient.channel_kind}/><span className="recipient-label"><strong>{recipient.channel_name}</strong><small>{service}{recipient.is_fallback?' · Fallback':''}</small></span><Icon className={`recipient-state${key==='processing'?' spin':''}`} size={17} aria-label={label}/></button>;
}

export function DeliveryLegend(){return <div className="delivery-legend" aria-label="Delivery status legend">{['sent','failed','retrying','queued','processing','cancelled','expired'].map(status=>{const {key,label,Icon}=deliveryState({status:status==='retrying'?'queued':status,error:status==='retrying'?'retry':null});return <span key={status} className={`recipient-${key}`}><Icon size={13} className="recipient-state"/>{label}</span>})}</div>}
