import test from 'node:test';
import assert from 'node:assert/strict';
import {spans,linkAt} from '../src/messageSyntax.js';

test('viewer only permits credential-free HTTP links',()=>{
 for(const raw of ['javascript:alert(1)','data:text/html,evil','file:///etc/passwd','https://user:secret@example.org','https://example.org/a b','https://example.org/\\evil','https://example.org/\n']) {
  assert.equal(linkAt(`[click](${raw})`),null,raw);
 }
 assert.equal(linkAt('[job](https://example.org/jobs/(42)) tail').url,'https://example.org/jobs/(42)');
 assert.equal(linkAt('[job](http://example.org)').url,'http://example.org/');
 assert.equal(linkAt('[](https://example.org)'),null);
});
test('literal HTML, unpaired delimiters and escaped formatting stay text',()=>{
 assert.deepEqual(spans('<script>alert(1)</script>'),[['<script>alert(1)</script>',false]]);
 assert.deepEqual(spans('**unclosed'),[['**',false],['unclosed',false]]);
 assert.deepEqual(spans('a **bold** \\*literal\\*'),[['a ',false],['bold',true],[' *literal*',false]]);
});
