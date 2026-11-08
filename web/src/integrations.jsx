import React,{useState} from 'react';
import {Webhook, Plug} from 'lucide-react';

export const groups = [
 {name:'Chat & Messaging', kinds:['telegram','slack','discord','teams','google_chat','matrix','mattermost','rocket_chat']},
 {name:'Push Notifications', kinds:['ntfy','pushover','gotify']},
 {name:'Email', kinds:['smtp']},
 {name:'Other', kinds:['webhook']},
];
export const integrations = {
 telegram:{label:'Telegram',description:'Send notifications via Telegram bot',help:'Topic ID is optional. Enter the numeric forum topic ID to post into a specific Telegram topic.',fields:[{key:'bot_token',label:'Bot token',secret:true},{key:'chat_id',label:'Chat ID',placeholder:'-1001234567890'},{key:'message_thread_id',label:'Topic ID',type:'number',max:2147483647,optional:true,placeholder:'42'}]},
 slack:{label:'Slack',description:'Send notifications to a Slack channel via webhook'},
 discord:{label:'Discord',description:'Send notifications to a Discord channel via webhook'},
 teams:{label:'Microsoft Teams',description:'Send notifications via a compatible Teams webhook'},
 google_chat:{label:'Google Chat',description:'Send notifications to a Google Chat space via webhook'},
 matrix:{label:'Matrix',description:'Send notifications to a Matrix room (Element, etc.)',help:'Invite the bot account to a room with encryption disabled. Enter the room ID, not its #alias. Encrypted rooms are not supported.',fields:[{key:'url',label:'Homeserver URL',secret:true,placeholder:'https://matrix.example.org'},{key:'access_token',label:'Access token',secret:true},{key:'room_id',label:'Room ID',placeholder:'!room:example.org'}]},
 mattermost:{label:'Mattermost',description:'Send via webhook or bot API, to a channel or direct message',help:'Use a webhook URL, or a bot / personal access token. Add the sender to its channel. Direct messages can target a user or bot by @username or user ID.'},
 rocket_chat:{label:'Rocket.Chat',description:'Send notifications to a Rocket.Chat channel via webhook'},
 ntfy:{label:'Ntfy',description:'Send push notifications via ntfy (self-hostable)',fields:[{key:'url',label:'Server URL',secret:true,placeholder:'https://ntfy.sh'},{key:'topic',label:'Topic',placeholder:'my-alerts'}]},
 pushover:{label:'Pushover',description:'Send push notifications via Pushover',fields:[{key:'token',label:'Application API token',secret:true},{key:'user',label:'User / group key',secret:true}]},
 gotify:{label:'Gotify',description:'Send push notifications via Gotify (self-hostable)',fields:[{key:'url',label:'Server URL',secret:true,placeholder:'https://gotify.example.org'},{key:'token',label:'Application token',secret:true}]},
 smtp:{label:'Email (SMTP)',description:'Send notifications via email',help:'Use STARTTLS (usually 587) or TLS (usually 465). Leave username and password empty for a relay without authentication. Plain SMTP requires ALLOW_INSECURE_HTTP=true on the server.',fields:[{key:'host',label:'SMTP host',placeholder:'smtp.example.org'},{key:'port',label:'Port',type:'number',value:'587'},{key:'tls',label:'Transport security',value:'starttls',options:[['starttls','STARTTLS'],['tls','TLS'],['none','None (trusted LAN only)']]},{key:'username',label:'Username',optional:true},{key:'password',label:'Password',secret:true,optional:true},{key:'from_name',label:'Sender name',optional:true,maxLength:100,placeholder:'Relayix Alerts'},{key:'from',label:'From address',placeholder:'Relayix <alerts@example.org>'},{key:'to',label:'Recipient address',placeholder:'ops@example.org'}]},
 webhook:{label:'Webhook',description:'Send notifications to any HTTP endpoint'},
};
const webhookFields=[{key:'url',label:'Webhook URL',secret:true,placeholder:'https://example.org/webhook'}];
export const types=groups.flatMap(g=>g.kinds);
const mattermostMode={key:'mode',label:'Delivery method',value:'webhook',options:[['webhook','Incoming webhook'],['bot','Bot / REST API']]};
function mattermostFields(config={}) {
 if(config.mode!=='bot')return [mattermostMode,...webhookFields];
 return [mattermostMode,{key:'url',label:'Server URL',secret:true,placeholder:'https://chat.example.org'},
  {key:'bot_token',label:'Bot / personal access token',secret:true},
  {key:'target',label:'Destination',value:'channel',options:[['channel','Channel'],['direct','Direct message']]},
  config.target==='direct'?{key:'recipient',label:'Recipient',placeholder:'@ops-bot or user ID'}:{key:'channel_id',label:'Channel ID',placeholder:'Copy from channel information'}];
}
function proxyFields(kind,config={}) {
 const mode=config.proxy_type||'none';
 const options=[['none','Direct connection'],['http','HTTP proxy'],...(kind==='telegram'?[['mtproto','MTProto proxy (plain / dd / ee)'],['webproxy','Telegram WEB-proxy']]:[])];
 const fields=[{key:'proxy_type',label:'Connection',value:'none',options}];
 if(mode==='http')fields.push({key:'proxy_host',label:'Proxy host',placeholder:'proxy.example.org'},{key:'proxy_port',label:'Proxy port',type:'number',value:'3128'},{key:'proxy_username',label:'Proxy login',optional:true,secret:true},{key:'proxy_password',label:'Proxy password',optional:true,secret:true});
 if(mode==='mtproto')fields.push({key:'proxy_host',label:'MTProxy host',placeholder:'proxy.example.org'},{key:'proxy_port',label:'MTProxy port',type:'number',value:'443'});
 if(mode==='webproxy')fields.push({key:'webproxy_address',label:'WEB-proxy address',placeholder:'proxy.example.org'});
 if(mode==='mtproto'||mode==='webproxy')fields.push({key:'proxy_secret',label:'Proxy secret',secret:true},{key:'api_id',label:'Telegram API ID',type:'number',max:2147483647,optional:true},{key:'api_hash',label:'Telegram API hash',secret:true,optional:true});
 return fields;
}
export const fieldsFor=(kind,config={})=>[...(kind==='mattermost'?mattermostFields(config):(integrations[kind]?.fields||webhookFields)),...proxyFields(kind,config)];
export const configFor=(kind,seed={})=>({...Object.fromEntries(fieldsFor(kind,seed).map(f=>[f.key,f.value||''])),...seed});
export const channelMethod=channel=>{
 const label=integrations[channel.kind]?.label||channel.kind,cfg=channel.config||{};
 if(channel.kind==='mattermost')return cfg.mode==='bot'?`${label} · Bot API · ${cfg.target==='direct'?'DM '+(cfg.recipient||''):'Channel'}`:`${label} · Webhook`;
 if(channel.kind==='telegram'&&cfg.message_thread_id)return `${label} · Topic ${cfg.message_thread_id}`;
 return label;
};

// Local SVG artwork: no remote images, icon fonts or third-party CDN requests.
export function ProviderIcon({kind}) {
 let drawing;
 switch(kind){
 case 'telegram':drawing=<><circle cx="16" cy="16" r="15" fill="#27a7e7"/><path d="m7 15 18-7-3 17-6-5-3 3v-5l9-7-11 6Z" fill="white"/></>;break;
 case 'matrix':drawing=<><path d="M6 2H2v28h4M26 2h4v28h-4" stroke="currentColor" fill="none"/><path d="M8 22V11h3v2c1-3 5-3 6 0 2-3 7-3 7 2v7h-3v-6c0-3-3-3-3 0v6h-3v-6c0-3-3-3-3 0v6Z" fill="currentColor"/></>;break;
 case 'mattermost':return <span className="provider provider-mattermost" aria-hidden="true"><img src="/providers/mattermost.svg" width="30" height="30" alt=""/></span>;
 case 'rocket_chat':drawing=<><path d="M4 5h18a8 8 0 0 1 8 8v5a8 8 0 0 1-8 8H12l-9 5 2-8a10 10 0 0 1-1-18Z" fill="#f53f62"/><circle cx="10" cy="16" r="2" fill="white"/><circle cx="17" cy="16" r="2" fill="white"/><circle cx="24" cy="16" r="2" fill="white"/></>;break;
 case 'ntfy':drawing=<><rect x="2" y="3" width="28" height="23" rx="6" fill="#55a65d"/><path d="m9 25-2 6 10-6" fill="#55a65d"/><path d="M10 20v-7a6 6 0 0 1 12 0v7l2 2H8Zm4 5h4" fill="white"/><circle cx="16" cy="8" r="2" fill="white"/></>;break;
 case 'pushover':drawing=<><rect x="2" y="2" width="28" height="28" rx="7" fill="#249df1"/><path d="M11 24V8h7a6 6 0 0 1 0 12h-3v4Zm4-8h3a2 2 0 0 0 0-4h-3Z" fill="white"/></>;break;
 case 'gotify':drawing=<><circle cx="16" cy="16" r="15" fill="#4386e8"/><path d="M13 6h6v10h6l-9 10-9-10h6Z" fill="white"/></>;break;
 case 'slack':drawing=<><path d="M12 3v11M3 12h6" stroke="#36c5f0" strokeWidth="6" strokeLinecap="round"/><path d="M29 12H18M20 3v6" stroke="#2eb67d" strokeWidth="6" strokeLinecap="round"/><path d="M20 29V18M29 20h-6" stroke="#ecb22e" strokeWidth="6" strokeLinecap="round"/><path d="M3 20h11M12 29v-6" stroke="#e01e5a" strokeWidth="6" strokeLinecap="round"/></>;break;
 case 'discord':drawing=<><path d="m7 6 6-2 1 3h4l1-3 6 2c5 7 6 13 5 18l-7 4-2-4c-4 1-6 1-10 0l-2 4-7-4C1 19 2 13 7 6Z" fill="#5865f2"/><ellipse cx="11" cy="16" rx="2.5" ry="3" fill="white"/><ellipse cx="22" cy="16" rx="2.5" ry="3" fill="white"/></>;break;
 case 'teams':drawing=<><circle cx="22" cy="7" r="5" fill="#7b83eb"/><rect x="13" y="14" width="18" height="15" rx="5" fill="#7b83eb"/><rect x="1" y="9" width="20" height="21" rx="3" fill="#5059c9"/><path d="M6 15h10m-5 0v10" stroke="white" strokeWidth="2.5"/></>;break;
 case 'google_chat':drawing=<><path d="M2 3h27v20H15l-8 7v-7H2Z" fill="#34a853"/><path d="M8 8h24v19H19l-5 5V15H8Z" fill="#188038"/><path d="M8 8h16v10H8Z" fill="#a8dab5"/></>;break;
 case 'smtp':drawing=<><rect x="1" y="4" width="30" height="24" rx="5" fill="#ef493d"/><path d="m2 8 14 10L30 8" stroke="white" strokeWidth="3" fill="none" strokeLinejoin="round"/></>;break;
 case 'webhook':return <span className="provider provider-webhook" aria-hidden="true"><Webhook size={29}/></span>;
 default:return <span className="provider" aria-hidden="true"><Plug size={26}/></span>;
 }
 return <span className={`provider provider-${kind}`} aria-hidden="true"><svg viewBox="0 0 32 32" width="30" height="30">{drawing}</svg></span>;
}

export function IntegrationCatalog({onSelect}) {
 return <div className="integration-catalog">{groups.map(group=><section className="integration-group" key={group.name} aria-label={group.name}>
  <h3>{group.name}</h3><div className="integration-grid">{group.kinds.map(kind=><button key={kind} className="integration" onClick={()=>onSelect(kind)}>
   <ProviderIcon kind={kind}/><span className="integration-copy"><strong>{integrations[kind].label}</strong><small>{integrations[kind].description}</small></span>
  </button>)}</div>
 </section>)}</div>;
}

function WebhookBodyField({draft,setDraft}) {
 const [text,setText]=useState(()=>draft.config.body_template?JSON.stringify(draft.config.body_template,null,2):'');
 const change=value=>{
  setText(value);
  try{
   const config={...draft.config};
   if(!value.trim())delete config.body_template;
   else{
    const parsed=JSON.parse(value);
    if(!parsed||typeof parsed!=='object'||Array.isArray(parsed))throw new Error('Use a JSON object for the webhook body.');
    config.body_template=parsed;
   }
   setDraft({...draft,config,body_template_error:''});
  }catch(e){setDraft({...draft,body_template_error:e.message})}
 };
 return <><label htmlFor="webhook-body-template">JSON body template (optional)</label><textarea id="webhook-body-template" rows={8} maxLength={16384} value={text} onChange={e=>change(e.target.value)} placeholder={'{"text":"{{message}}","title":"{{title}}","priority":"{{priority}}"}'}/><p className="helptext">Leave empty for the standard payload. Variables: {'{{title}}'}, {'{{message}}'}, {'{{priority}}'}, {'{{source}}'} in JSON values. A whole-value priority variable stays numeric. Maximum template size: 16 KiB. Do not put credentials in body templates; they are visible to authenticated users and excluded from configuration exports.</p>{draft.body_template_error&&<p className="field-error">{draft.body_template_error}</p>}</>;
}

export function ChannelFields({draft,setDraft}) {
 const fields=fieldsFor(draft.kind,draft.config),advanced=field=>['api_id','api_hash'].includes(field.key);
 const renderField=field=><React.Fragment key={field.key}>
  <label htmlFor={`config-${field.key}`}>{field.label}{field.optional?' (optional)':''}</label>
  {field.options?<select id={`config-${field.key}`} value={draft.config[field.key]||field.value} onChange={e=>{const value=e.target.value;if(field.key==='proxy_type')setDraft({...draft,config:configFor(draft.kind,{...draft.config,proxy_type:value})});else if(draft.kind==='mattermost'&&field.key==='mode')setDraft({...draft,config:configFor('mattermost',{mode:value})});else setDraft({...draft,config:{...draft.config,[field.key]:value}})}}>
   {field.options.map(([value,label])=><option key={value} value={value}>{label}</option>)}
  </select>:<input id={`config-${field.key}`} type={field.secret?'password':field.type||'text'} autoComplete={field.secret?'new-password':'off'}
   value={draft.config[field.key]??field.value??''} maxLength={field.maxLength} required={!field.optional} min={field.type==='number'?1:undefined} max={field.type==='number'?(field.max||65535):undefined}
   placeholder={field.placeholder||''} onChange={e=>setDraft({...draft,config:{...draft.config,[field.key]:e.target.value}})}/>}
 </React.Fragment>;
 return <>{['telegram','discord'].includes(draft.kind)&&<p className="helptext">Messages exceeding the provider limit are truncated with a visible marker. Full incoming content remains in history; Telegram formatting is rendered safely after truncation.</p>}{fields.filter(field=>!advanced(field)).map(renderField)}{draft.kind==='webhook'&&<WebhookBodyField draft={draft} setDraft={setDraft}/>}{fields.some(advanced)&&<details className="telegram-advanced"><summary>Application credentials (optional override)</summary><p className="helptext">Leave empty to use the server's Telegram application credentials. These belong to the sending application, not your proxy provider.</p>{fields.filter(advanced).map(renderField)}</details>}{integrations[draft.kind]?.help&&<p className="helptext integration-help">{integrations[draft.kind].help}</p>}{draft.config.proxy_type==='http'&&<p className="helptext">This channel uses the HTTP proxy for every request. HTTPS and SMTP use CONNECT. Authentication is optional; provide both login and password when required. Proxy failure never falls back to a direct connection.</p>}{['mtproto','webproxy'].includes(draft.config.proxy_type)&&<p className="helptext">Enter the address, port and secret supplied by your proxy provider. The server administrator configures Telegram application credentials once for MTProto/WEB delivery. Use a numeric Chat ID or @username; no chat access hash is needed for bots. MTProxy supports plain, dd and ee secrets, including telemt. WEB-proxy supports plain/dd and a root HTTPS hostname on port 443.</p>}</>;
}
