import React from 'react';
import {Tag} from 'lucide-react';
import {ProviderIcon,channelMethod} from './integrations.jsx';

export function BrandMark(){return <span className="brand-mark" aria-hidden="true"><img src="/relayix-mark.svg" width="44" height="44" alt=""/></span>}
export function BrandName(){return <>Relayix <span className="beta-badge">Beta</span></>}
export function RecipientBadges({ids,channels}) {
 return <div className="recipient-badges" aria-label="Recipients">{ids.map(id=>{
  const channel=channels.find(c=>c.id===id);
  if(!channel)return <span className="recipient-badge muted" key={id}>Removed channel</span>;
  const method=channelMethod(channel);
  return <span className={`recipient-badge ${channel.enabled?'':'recipient-disabled'}`} key={id} data-kind={channel.kind} title={`${channel.name} · ${method}${channel.enabled?'':' · Disabled'}`} aria-label={`Recipient ${channel.name}, ${method}`}><ProviderIcon kind={channel.kind}/><span className="recipient-copy"><strong>{channel.name}</strong><small>{method}{channel.enabled?'':' · Disabled'}</small></span></span>;
 })}{!ids.length&&<span className="muted">No recipients selected</span>}</div>;
}
export function RouteTag({tag,onClick}) {
 if(!tag)return null;
 return onClick?<button type="button" className="chip tag-chip route-tag" title={tag} aria-label={`Filter tag ${tag}`} onClick={()=>onClick(tag)}><Tag size={12}/>{tag}</button>:<span className="chip route-tag" title={tag}><Tag size={12}/>{tag}</span>;
}
export function RouteTagFilter({label,routes,value,onChange}) {
 const values=[...new Set(routes.map(r=>r.required_tag).filter(Boolean))].sort();
 return <div className="route-tag-filter"><label className="form-field"><span>{label}</span><select aria-label={label} value={value} onChange={e=>onChange(e.target.value)}><option value="">All tags</option>{values.map(tag=><option value={tag} key={tag}>{tag}</option>)}{value&&!values.includes(value)&&<option value={value}>{value} (no matching routes)</option>}</select></label>{value&&<button className="secondary compact" onClick={()=>onChange('')}>Clear tag filter</button>}</div>;
}
