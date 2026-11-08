import React from 'react';

import {spans,linkAt} from './messageSyntax.js';

function links(text) {
 const result=[];let literal='';
 for(let i=0;i<text.length;){
  const rest=text.slice(i);
  if(rest.startsWith('\\[')||rest.startsWith('\\]')){literal+=rest[1];i+=2;continue;}
  const link=rest.startsWith('[')?linkAt(rest):null;
  if(link){if(literal){result.push(literal);literal=''}result.push(<a key={i} href={link.url} target="_blank" rel="noopener noreferrer">{link.label}</a>);i+=link.length;}
  else{literal+=text[i];i++;}
 }
 if(literal)result.push(literal);
 return result;
}
export function MessageBody({text=''}) {
 return <div className="message-view" aria-label="Full message">{spans(text).map(([body,bold],i)=>bold?<strong key={i}>{links(body)}</strong>:<React.Fragment key={i}>{links(body)}</React.Fragment>)}</div>;
}
