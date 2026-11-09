import test from 'node:test';
import assert from 'node:assert/strict';
import {integrationExamples,shellQuote} from '../src/integrationExamples.js';

test('examples include the actual route and safe JSON for quotes and command substitution',()=>{
 const endpoint='https://relay.example/h/route-123';
 const examples=integrationExamples(endpoint,"Build's `result`",'$(touch /tmp/example)\n😀','YOUR_ROUTE_KEY');
 for(const source of Object.values(examples)){assert.ok(source.includes(endpoint));assert.ok(source.includes('X-Relay-Key'));assert.ok(source.includes('YOUR_ROUTE_KEY'))}
 assert.ok(examples.python.includes('urllib.request'));
 assert.ok(examples.bash.includes('set -euo pipefail'));
 assert.equal(shellQuote("a'b"),"'a'\"'\"'b'");
 assert.ok(examples.curl.includes('\\n'));
});
test('custom keys stay in the returned examples and defaults remain placeholders',()=>{
 assert.ok(integrationExamples('https://example/h/test','','Body').curl.includes('YOUR_ROUTE_KEY'));
 assert.ok(integrationExamples('https://example/h/test','','Body','custom-value').python.includes('custom-value'));
});
