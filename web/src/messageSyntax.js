// Match the shared delivery syntax with React nodes; raw HTML stays literal.
export function spans(text) {
 const tokens=[];let plain='';
 for(let i=0;i<text.length;i++){
  const c=text[i],next=text[i+1];
  if(c==='\\'&&next==='\n'){plain+='\n';i++;}
  else if(c==='\\'&&(next==='*'||next==='\\')){plain+=next;i++;}
  else if(c==='*'&&next==='*'){if(plain){tokens.push(plain);plain=''}tokens.push(null);i++;}
  else plain+=c;
 }
 if(plain)tokens.push(plain);
 const result=[];
 for(let i=0;i<tokens.length;i++){
  if(tokens[i]!==null){result.push([tokens[i],false]);continue;}
  const end=tokens.indexOf(null,i+1);
  if(end>i+1){const body=tokens.slice(i+1,end).join('');if(body.trim()){result.push([body,true]);i=end;continue;}}
  result.push(['**',false]);
 }
 return result;
}
export function linkAt(text) {
 const close=text.indexOf('](');
 if(close<0||!text.slice(1,close).trim())return null;
 const start=close+2;let depth=1;
 for(let i=start;i<text.length;i++){
  if(text[i]==='(')depth++;
  if(text[i]===')')depth--;
  if(depth!==0)continue;
  const raw=text.slice(start,i);
  if(/[\s\\\u0000-\u001f\u007f]/u.test(raw))return null;
  try{const url=new URL(raw);if(!['http:','https:'].includes(url.protocol)||!url.hostname||url.username||url.password)return null;return {length:i+1,label:text.slice(1,close),url:url.href};}catch{return null;}
 }
 return null;
}
