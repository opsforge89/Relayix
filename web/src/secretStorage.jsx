import React,{useEffect,useState} from 'react';
import {ErrorMessage} from './ui.jsx';
export function SecretStorage({api}) {
 const [state,setState]=useState(null),[error,setError]=useState('');
 useEffect(()=>{let active=true;api('/api/security/storage').then(value=>{if(active)setState(value)}).catch(e=>{if(active)setError(e.message)});return()=>{active=false}},[api]);
 return <section className="panel"><h2>Stored credential encryption</h2><ErrorMessage error={error}/>{state?<><p role="status">{state.enabled?'Database credentials protected with AES-256-GCM':'Database credential encryption disabled'}</p><p className="helptext">{state.enabled?'Keep SECRETS_KEY outside the database. Losing it prevents decryption; an encrypted full backup can recover it.':'Configure a random 32-byte SECRETS_KEY in the server environment and restart all replicas to migrate existing channel and authenticator secrets.'} Credential fields stay masked in the interface; the audit log contains no secret values. Full backups use a separate password.</p></>:<p>Loading credential storage status…</p>}</section>;
}
