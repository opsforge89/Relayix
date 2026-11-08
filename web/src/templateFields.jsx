import React,{useState} from 'react';
import {Search,LayoutGrid,List} from 'lucide-react';

export const fieldGroups=[
 {name:'Message',description:'Content and routing values from your incoming notification.',fields:[['message','Message text','Deployment completed'],['title','Notification heading','Production deployment'],['tags','Labels supplied as an array',['deploy','production']],['priority','Notification priority, from 1 to 5',3]]},
 {name:'Tests',description:'Test results supplied by your test runner.',fields:[['test_type','Type of tests','UnitTests'],['suite','Test suite name','ExampleSuite'],['status','Result or status text','Failed'],['failed','Number of failed tests',2],['passed','Number of passed tests',40],['skipped','Number of skipped tests',0],['log_url','Link to the runner log','https://gitlab.example.org/acme/demo/-/jobs/12345']]},
 {name:'Deployment',description:'Project and build values supplied by your CI/CD integration.',fields:[['project','Project or service name','ExampleService'],['branch','Git branch','release/1.0'],['environment','Deployment environment','production'],['version','Release version','1.0.0'],['commit','Commit identifier','a1b2c3d'],['pipeline_url','Link to the pipeline','https://gitlab.example.org/acme/demo/-/pipelines/42'],['build.id','Build identifier inside a nested build object','12345'],['build.branch','Branch inside a nested build object','main']]},
 {name:'Jobs',description:'Scheduled jobs and backup reports supplied by your automation.',fields:[['host','Host running the job','db-01.example.org'],['job_name','Job or backup name','Daily database backup'],['finished_at','Job completion time','2026-11-07T10:05:00Z'],['duration','Elapsed time','5 minutes'],['backup_size','Backup size','2.4 GB'],['storage_url','Link to the backup location','https://storage.example.org/backups/latest']]},
];
export function collectPayloadFields(payload,prefix='',depth=0,result=[]){
 if(depth>8||!payload||typeof payload!=='object'||Array.isArray(payload))return result;
 for(const [key,value] of Object.entries(payload)){
  const path=prefix?`${prefix}.${key}`:key;
  if(!/^[a-zA-Z0-9_]+$/.test(key)||result.length>=200)continue;
  if(value&&typeof value==='object'&&!Array.isArray(value))collectPayloadFields(value,path,depth+1,result);
  else result.push([path,'Field from your example JSON',value]);
 }
 return result;
}
export function TemplateFields({example,onInsert,computedStatus=false}){
 const [category,setCategory]=useState('Message'),[search,setSearch]=useState(''),[layout,setLayout]=useState('cards');
 let payload={},invalid=false;try{payload=JSON.parse(example)}catch{invalid=true}
 const own=collectPayloadFields(payload),available=new Map(own.map(([key,,value])=>[key,value]));
 const all=[...new Map([...fieldGroups.flatMap(group=>group.fields),...own.filter(([key])=>!fieldGroups.some(group=>group.fields.some(([known])=>known===key)))].map(field=>[field[0],field])).values()];
 const groups=[...fieldGroups,{name:'All',description:'All suggested fields plus fields discovered in your example JSON, without duplicates. Nested objects use dotted paths.',fields:all}];
 const group=groups.find(g=>g.name===category)||groups[0];
 const fields=(search?all:group.fields).filter(([key,description])=>`${key} ${description}`.toLowerCase().includes(search.toLowerCase()));
 return <section className="field-browser" aria-label="Template field library"><div className="field-browser-heading"><h3>Insert a field</h3><div className="field-layout" role="group" aria-label="Field display"><button type="button" aria-label="Cards with examples" aria-pressed={layout==='cards'} onClick={()=>setLayout('cards')}><LayoutGrid size={15}/></button><button type="button" aria-label="Compact field list" aria-pressed={layout==='list'} onClick={()=>setLayout('list')}><List size={15}/></button></div></div><label className="field-search"><Search size={15}/><input aria-label="Search template fields" placeholder="Search fields…" value={search} onChange={e=>setSearch(e.target.value)}/></label><div className="field-categories" role="group" aria-label="Field categories">{groups.map(g=><button type="button" key={g.name} aria-pressed={category===g.name} onClick={()=>{setCategory(g.name);setSearch('')}}>{g.name} <small>{g.fields.length}</small></button>)}</div><p className="helptext">{search?'Matching fields across all categories.':group.description}</p><p className="helptext">Click to insert at the cursor in the last focused title, message or link URL. Fields must be supplied in incoming JSON; examples are suggestions.</p>{invalid&&<p className="field-error">Fix the example JSON to detect available fields.</p>}<div className={`field-options ${layout}`}>
 {fields.map(([key,description,sample])=><button className="field-option" type="button" key={key} onClick={()=>onInsert(key)}><span className="field-option-key"><code>{`{{${key}}}`}</code><small className={available.has(key)||(computedStatus&&key==='status')?'field-available':'field-suggested'}>{computedStatus&&key==='status'?'Computed':available.has(key)?'In JSON':'Suggested'}</small></span><span>{description}</span><small className="field-sample">Example: {computedStatus&&key==='status'?'❌ / ✅ / ⚠️':JSON.stringify(available.has(key)?available.get(key):sample)}</small></button>)}
 {!fields.length&&<p className="helptext">{search?'No matching fields.':'Paste your JSON on the right to discover your own fields.'}</p>}
 </div></section>;
}
