import React,{useLayoutEffect,useId,useRef,useState} from 'react';
import {MoreVertical,Send,Activity,Pause,Play,Pencil,Trash2,CopyPlus,KeyRound,Copy,Users,ArrowRight} from 'lucide-react';
import {Dialog,ErrorMessage,RouteUsageContent} from './ui.jsx';
import {ChannelAccessForm} from './channelAccess.jsx';
import {ProviderIcon,channelMethod} from './integrations.jsx';
import {RecipientBadges,RouteTag} from './brand.jsx';
import {RouteIcon} from './routeIcons.jsx';
import {InlineDiagnostics} from './operations.jsx';
import {manuallyPaused} from './deliveryControls.jsx';

export function ActionMenu({label,items}) {
 const [open,setOpen]=useState(false),[position,setPosition]=useState({});
 const ref=useRef(null),trigger=useRef(null),menu=useRef(null),id=useId();
 useLayoutEffect(()=>{
  if(!open)return;
  const place=()=>{
   const rect=trigger.current.getBoundingClientRect(),height=menu.current.offsetHeight;
   if(rect.bottom<0||rect.top>window.innerHeight){setOpen(false);return}
   setPosition({left:Math.max(8,Math.min(rect.right-220,window.innerWidth-228)),top:rect.bottom+height+8>window.innerHeight?Math.max(8,rect.top-height-4):rect.bottom+4});
  };
  place();
  menu.current.querySelector('button:not(:disabled)')?.focus({preventScroll:true});
  const outside=e=>{if(!ref.current?.contains(e.target))setOpen(false)};
  const scroll=e=>{if(!menu.current?.contains(e.target))place()};
  document.addEventListener('pointerdown',outside);window.addEventListener('resize',scroll);window.addEventListener('scroll',scroll,true);
  return()=>{document.removeEventListener('pointerdown',outside);window.removeEventListener('resize',scroll);window.removeEventListener('scroll',scroll,true)};
 },[open]);
 if(!items.length)return null;
 const close=()=>{setOpen(false);trigger.current?.focus({preventScroll:true})};
 const keys=e=>{
  if(e.key==='Escape'){e.preventDefault();e.stopPropagation();close()}
  if(e.key==='Tab'){setOpen(false);return}
  if(['ArrowDown','ArrowUp','Home','End'].includes(e.key)){
   e.preventDefault();const buttons=[...menu.current.querySelectorAll('button:not(:disabled)')],index=buttons.indexOf(document.activeElement);
   const next=e.key==='Home'?0:e.key==='End'?buttons.length-1:(index+(e.key==='ArrowDown'?1:-1)+buttons.length)%buttons.length;
   buttons[next]?.focus();
  }
 };
 return <div className="resource-actions" ref={ref} onKeyDown={open?keys:undefined}><button ref={trigger} className="ghost" type="button" aria-label={label} aria-haspopup="menu" aria-expanded={open} aria-controls={id} onClick={()=>setOpen(!open)} onKeyDown={e=>{if(e.key==='ArrowDown'){e.preventDefault();setOpen(true)}}}><MoreVertical size={18}/></button>{open&&<div ref={menu} id={id} className="resource-action-menu" role="menu" aria-label={label} style={position}>{items.map(({label,ariaLabel,icon:Icon,action,disabled,danger})=><button key={label} role="menuitem" type="button" className={danger?'danger':''} aria-label={ariaLabel||label} disabled={disabled} onClick={()=>{close();action()}}>{Icon&&<Icon size={16}/>}<span>{label}</span></button>)}</div>}</div>;
}

export const canEditChannel=channel=>channel.permissions?.edit&&channel.config?.body_template!=='********';
export function channelState(channel) {
 if(!channel.enabled)return {label:'Disabled',className:'disabled'};
 if(manuallyPaused(channel))return {label:'Paused manually',className:'queued'};
 if(channel.health?.state==='paused')return {label:'Paused',className:'queued'};
 if(channel.health?.state==='probe')return {label:'Probe pending',className:'queued'};
 return {label:'Active',className:'sent'};
}
export function ChannelStatus({channel}) {const state=channelState(channel);return <span className={`status ${state.className}`}>{state.label}</span>}
export function ChannelActions({channel,busy,onTest,onEdit,onDelete,onDiagnose,onPause,onResume,onAccess,diagnostics}) {
 const p=channel.permissions||{},editable=canEditChannel(channel),paused=manuallyPaused(channel)||['paused','probe'].includes(channel.health?.state);
 const items=[];
 if(p.send)items.push({label:'Send test',icon:Send,disabled:busy,action:()=>onTest(channel.id)});
 if(p.edit)items.push({label:'Diagnostics',icon:Activity,disabled:busy||diagnostics[channel.id]?.busy,action:()=>onDiagnose(channel)});
 if(editable)items.push({label:paused?'Resume':'Pause',icon:paused?Play:Pause,disabled:busy,action:()=>paused?onResume(channel):onPause(channel)},{label:'Edit',icon:Pencil,disabled:busy,action:()=>onEdit(channel)});
 if(p.manage)items.push({label:'Access',icon:Users,disabled:busy,action:()=>onAccess(channel)},{label:'Delete channel',ariaLabel:`Delete channel ${channel.name}`,icon:Trash2,danger:true,disabled:busy||channel.config?.body_template==='********',action:()=>onDelete(channel)});
 return <ActionMenu label={`Channel actions ${channel.name}`} items={items}/>;
}
export function RouteActions({route,busy,onTest,onClone,onEdit,onRotate,onDelete,onUsage,onCopy}) {
 const manage=route.can_manage!==false;
 return <ActionMenu label={`Route actions ${route.name}`} items={[
  {label:'Test route',ariaLabel:`Test webhook ${route.name}`,icon:Send,disabled:busy||!manage,action:()=>onTest(route)},
  {label:'Integration guide',icon:Copy,action:()=>onUsage(route)},
  {label:'Copy endpoint',ariaLabel:`Copy endpoint ${route.name}`,icon:Copy,action:()=>onCopy(`${location.origin}/h/${route.slug}`)},
  {label:'Clone',ariaLabel:`Clone route ${route.name}`,icon:CopyPlus,disabled:busy,action:()=>onClone(route)},
  ...(manage?[
   {label:'Edit',ariaLabel:`Edit route ${route.name}`,icon:Pencil,disabled:busy,action:()=>onEdit(route)},
   {label:'Rotate key',ariaLabel:`Rotate key ${route.name}`,icon:KeyRound,disabled:busy,action:()=>onRotate(route.id)},
   {label:'Delete route',ariaLabel:`Delete route ${route.name}`,icon:Trash2,danger:true,disabled:busy,action:()=>onDelete(route)},
  ]:[]),
 ]}/>;
}

export function ChannelCard({channel,onView,compact=false,...props}) {
 const cfg=channel.config||{},destination=({telegram:cfg.chat_id,mattermost:cfg.target==='direct'?cfg.recipient:cfg.channel_id,matrix:cfg.room_id,smtp:cfg.to,ntfy:cfg.topic})[channel.kind];
 const summary=[channelMethod(channel),!compact&&destination&&destination!=='********'?destination:null,!compact?({http:'HTTP proxy',mtproto:'MTProxy',webproxy:'WEB-proxy'})[cfg.proxy_type]||'Direct':null].filter(Boolean).join(' · ');
 return <div className="channel-entry"><article className={`channel-card resource-row${compact?' compact-resource':''}`}><div className="item-main"><ProviderIcon kind={channel.kind}/><div className="channel-copy"><button className="link resource-name" onClick={()=>onView(channel)}>{channel.name}</button><small>{summary}</small></div></div><ChannelStatus channel={channel}/><ChannelActions channel={channel} {...props}/></article>{!compact&&manuallyPaused(channel)&&<p className="helptext">{channel.manual_pause_until?`Paused until ${new Date(channel.manual_pause_until).toLocaleString()}.`:'Paused until you resume.'} Queued notifications and alerts are retained.</p>}{!compact&&channel.enabled&&['paused','probe'].includes(channel.health?.state)&&<p className="helptext">{channel.health.last_error} Queued messages are retained.</p>}</div>;
}
export function RouteRow({route,channels,onView,onTag,compact=false,...props}) {
 return <article className={`route-row resource-row${compact?' compact-resource':''}`}><div className="resource-route-copy"><div className="route-layout"><div className="route-card-heading"><div className="route-name"><span className="route-symbol"><RouteIcon name={route.icon} size={21}/></span><button className="link resource-name" title={route.name} onClick={()=>onView(route)}>{route.name}</button></div></div><ArrowRight className="route-flow-arrow" size={16} aria-hidden="true"/><div className="route-recipients"><RecipientBadges ids={route.channels.slice(0,3)} channels={channels}/>{route.channels.length>3&&<span className="route-more" aria-label={`${route.channels.length-3} more recipients`} title={`${route.channels.length-3} more recipients`}>+{route.channels.length-3}</span>}<RouteTag tag={route.required_tag} onClick={onTag}/></div></div></div><span className={`status ${route.enabled?'sent':'disabled'}`}>{route.enabled?'Active':'Disabled'}</span><RouteActions route={route} {...props}/></article>;
}

function DetailTabs({tabs,value,onChange,id}) {
 const refs=useRef([]);
 return <div className="detail-tabs" role="tablist" aria-label="Detail sections">{tabs.map((tab,index)=><button key={tab} ref={el=>{refs.current[index]=el}} id={`${id}-${tab}`} role="tab" aria-controls={`${id}-panel`} aria-selected={value===tab} tabIndex={value===tab?0:-1} onClick={()=>onChange(tab)} onKeyDown={e=>{
  if(!['ArrowLeft','ArrowRight','Home','End'].includes(e.key))return;
  e.preventDefault();const next=e.key==='Home'?0:e.key==='End'?tabs.length-1:(index+(e.key==='ArrowRight'?1:-1)+tabs.length)%tabs.length;
  onChange(tabs[next]);refs.current[next]?.focus();
 }}>{tab}</button>)}</div>;
}
function Properties({items}) {return <dl className="resource-properties">{items.map(([label,value])=><React.Fragment key={label}><dt>{label}</dt><dd>{value??'None'}</dd></React.Fragment>)}</dl>}

export function ResourceDrawer({type,resource,tab,onTab,onClose,editor,channelProps,routeProps,api,isAdmin,busy,error,onSaved,onCopy,onRotate}) {
 const channel=type==='channel',p=resource.permissions||{},editable=channel?canEditChannel(resource):resource.can_manage!==false;
 const tabs=['Overview',...((channel?(p.view||editable):editable)?['Settings']:[]),...(channel?(p.manage?['Access']:[]):['Integration'])];
 const selected=tabs.includes(tab)?tab:'Overview';
 const [accessBusy,setAccessBusy]=useState(false),panelId=useId();
 return <Dialog title={channel?'Channel details':'Route details'} onClose={onClose} drawer className="resource-drawer" busy={busy||accessBusy}><div className="resource-identity">{channel?<ProviderIcon kind={resource.kind}/>:<span className="route-symbol"><RouteIcon name={resource.icon} size={26}/></span>}<div><h3>{resource.name}</h3>{channel&&<p>{channelMethod(resource)}</p>}</div>{channel?<ChannelStatus channel={resource}/>:<span className={`status ${resource.enabled?'sent':'disabled'}`}>{resource.enabled?'Active':'Disabled'}</span>}</div><DetailTabs id={panelId} tabs={tabs} value={selected} onChange={value=>{if(!busy&&!accessBusy)onTab(value)}}/><ErrorMessage error={error}/><div className="resource-detail-content" id={`${panelId}-panel`} role="tabpanel" aria-labelledby={`${panelId}-${selected}`} tabIndex={0}>
 {selected==='Overview'&&(channel?<><Properties items={[
  ['Connection',channelMethod(resource)],['Ownership',p.owner_user_id?'Personal channel':'Administrator-managed'],['Your permissions',p.manage?'Full access':[p.view&&'View',p.edit&&'Edit',p.send&&'Send'].filter(Boolean).join(', ')],
  ...(p.edit?[['Delivery state',resource.health?.state||'Ready'],['Recent error',resource.health?.last_error||'None']]:[]),
 ]}/>{p.view&&<p className="helptext">Open Settings to view connection parameters. Credentials are hidden.</p>}<InlineDiagnostics state={channelProps.diagnostics[resource.id]} onDismiss={()=>channelProps.onDismissDiagnostics(resource.id)}/></>:<><h3>Recipients</h3><RecipientBadges ids={resource.channels} channels={routeProps.channels}/><h3>Fallback</h3>{resource.fallback_channel_ids?.length?<RecipientBadges ids={resource.fallback_channel_ids} channels={routeProps.channels}/>:<p className="helptext">No fallback recipients.</p>}<Properties items={[
  ['Minimum priority',resource.min_priority],['Rate limit',resource.rate_limit_per_minute?`${resource.rate_limit_per_minute} events / minute`:'Unlimited'],['Tag',resource.required_tag||'None'],['Quiet hours',resource.delivery_settings?.quiet_enabled?resource.delivery_settings.timezone||'Enabled':'Disabled'],['Grouping',resource.delivery_settings?.dedup_seconds?`${resource.delivery_settings.dedup_seconds}s deduplication`:resource.delivery_settings?.digest_seconds?`${resource.delivery_settings.digest_seconds}s digest`:'None'],
 ]}/><p className="helptext">Open Integration for the permanent webhook endpoint and code examples.</p></>)}
 {selected==='Settings'&&(editable?editor:<><p className="helptext">Read-only settings. Credentials are hidden.</p><pre className="resource-config">{JSON.stringify(resource.config,null,2)}</pre></>)}
 {selected==='Access'&&p.manage&&<ChannelAccessForm channel={resource} api={api} isAdmin={isAdmin} onClose={onClose} onSaved={onSaved} onBusyChange={setAccessBusy}/>}
 {selected==='Integration'&&!channel&&<><RouteUsageContent route={resource} onCopy={onCopy}/>{editable&&<button className="secondary" disabled={busy} onClick={()=>onRotate(resource.id)}><KeyRound size={16}/> Rotate key</button>}</>}
 </div>{selected==='Overview'&&<div className="resource-detail-footer">{(channel?p.send:editable)&&<button className="primary" disabled={busy} onClick={()=>channel?channelProps.onTest(resource.id):routeProps.onTest(resource)}><Send size={16}/>{channel?'Send test':'Test route'}</button>}{channel?<ChannelActions channel={resource} {...channelProps}/>:<RouteActions route={resource} {...routeProps}/>}</div>}</Dialog>;
}
