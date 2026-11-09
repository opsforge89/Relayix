export const shellQuote=value=>"'"+String(value).replaceAll("'","'\"'\"'")+"'";
export function integrationExamples(endpoint,title,message,key='YOUR_ROUTE_KEY') {
 const payload=JSON.stringify({title,message});
 const curl=[`curl --fail-with-body --request POST ${shellQuote(endpoint)}`,"  --header 'Content-Type: application/json'",`  --header ${shellQuote('X-Relay-Key: '+key)}`,`  --data ${shellQuote(payload)}`].join(' \\\n');
 const python=`import json\nimport urllib.request\n\npayload = json.loads(${JSON.stringify(payload)})\nrequest = urllib.request.Request(\n    ${JSON.stringify(endpoint)},\n    data=json.dumps(payload).encode("utf-8"),\n    headers={"Content-Type": "application/json", "X-Relay-Key": ${JSON.stringify(key)}},\n    method="POST",\n)\nwith urllib.request.urlopen(request, timeout=15) as response:\n    print(response.read().decode("utf-8"))\n`;
 const bash=`#!/usr/bin/env bash\nset -euo pipefail\n\nendpoint=${shellQuote(endpoint)}\nroute_key=${shellQuote(key)}\npayload=${shellQuote(payload)}\n\ncurl --fail-with-body --request POST "$endpoint" \\\n  --header 'Content-Type: application/json' \\\n  --header "X-Relay-Key: $route_key" \\\n  --data "$payload"\n`;
 return {curl,python,bash};
}
