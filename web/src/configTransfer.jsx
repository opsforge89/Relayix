import React,{useRef,useState} from 'react';
import {ErrorMessage} from './ui.jsx';

const MAX_BYTES=2*1024*1024;
export function ConfigurationTransfer({api,onImported}) {
 const fileInput=useRef(null),[bundle,setBundle]=useState(null),[busy,setBusy]=useState(false),[error,setError]=useState(''),[notice,setNotice]=useState('');
 const download=async()=>{
  setBusy(true);setError('');setNotice('');
  try{
   const data=await api('/api/config/export'),blob=new Blob([JSON.stringify(data,null,2)+'\n'],{type:'application/json'}),url=URL.createObjectURL(blob),link=document.createElement('a');
   link.href=url;link.download='relayix-configuration.json';link.click();setTimeout(()=>URL.revokeObjectURL(url),1000);
   setNotice('Configuration exported without credentials, endpoint URLs, custom webhook bodies or route keys.');
  }catch(e){setError(e.message)}finally{setBusy(false)}
 };
 const select=async e=>{
  const file=e.target.files?.[0];setError('');setNotice('');setBundle(null);
  if(!file)return;
  try{
   if(file.size>MAX_BYTES)throw new Error('Configuration files must not exceed 2 MiB.');
   const data=JSON.parse(await file.text());
   if(data.schema_version!==1||!Array.isArray(data.channels)||!Array.isArray(data.routes))throw new Error('Choose a Relayix configuration export (schema version 1).');
   setBundle(data);
  }catch(e){setError(e.message)}
  e.target.value='';
 };
 const upload=async()=>{
  setBusy(true);setError('');setNotice('');
  try{
   const result=await api('/api/config/import',{method:'POST',body:JSON.stringify(bundle)});
   setBundle(null);setNotice(`Imported ${result.channels} channels and ${result.routes} routes as disabled drafts. Restore credentials and URLs, review settings, and rotate new route keys before enabling.`);
   await onImported();
  }catch(e){setError(e.message)}finally{setBusy(false)}
 };
 return <section className="configuration-transfer"><h2>Configuration transfer</h2><p className="helptext">Export channels and routes as JSON. Credentials, endpoint URLs, custom webhook bodies, message templates, original route keys and history are excluded. Review names and other metadata before sharing. Import appends disabled drafts with new IDs and keys; existing settings are preserved. Maximum: 500 channels, 500 routes and 2 MiB.</p><div className="settings-actions"><button className="secondary" disabled={busy} onClick={download}>Export configuration</button><button className="secondary" disabled={busy} onClick={()=>fileInput.current?.click()}>Choose configuration file</button><input ref={fileInput} type="file" accept="application/json,.json" hidden onChange={select}/></div>{bundle&&<div className="panel"><p>Ready to import {bundle.channels.length} channels and {bundle.routes.length} routes as disabled drafts.</p><button className="primary" disabled={busy} onClick={upload}>{busy?'Importing…':'Import disabled drafts'}</button><button className="secondary" disabled={busy} onClick={()=>setBundle(null)}>Cancel</button></div>}<ErrorMessage error={error}/>{notice&&<p role="status">{notice}</p>}</section>;
}
