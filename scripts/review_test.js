// Dependency-free regression tests for the browser's offline review logic.
const {test}=require('node:test');
const assert=require('node:assert/strict');
const core=require('../src/tools/review.js');
const agent=(id=1)=>({id,segment:0,x:2,y:3,team:'home',jersey:7,state:'visible',identity:{status:'candidate',player:{team:'home',number:7}}});
const rows=[{t:10,players:[agent()]},{t:11,players:[agent()]}];
const doc=()=>({schema_version:1,source_sha256:'test-hash',roster:[{id:'h7',team:'home',number:7,name:'Alex'}],corrections:[]});
test('binary search never uses future observations or fills large gaps',()=>{
  assert.equal(core.at(rows,9),null);assert.equal(core.at(rows,10.5),rows[0]);assert.equal(core.at(rows,11),rows[1]);assert.equal(core.at(rows,13),null);
});
test('input requires segments, finite positions and sorted times',()=>{
  assert.equal(core.frames(rows.map(JSON.stringify).join('\n')).length,2);
  assert.throws(()=>core.frames(JSON.stringify({t:0,players:[{id:1,x:2,y:3}]})));
  assert.throws(()=>core.frames([...rows].reverse().map(JSON.stringify).join('\n')));
});
test('corrections are immutable, scoped, and exportable',()=>{
  const d=doc();d.corrections=[{slot:1,segment:0,start:10,end:11,player_id:null}];core.validate(d,'test-hash',rows);
  assert.equal(core.preview(rows[0],d)[0].identity.status,'unknown');
  assert.equal(core.preview(rows[1],d)[0].jersey,7);assert.equal(rows[0].players[0].jersey,7);
  assert.throws(()=>core.validate(d,'wrong-hash',rows));
  d.corrections[0].segment=2;assert.throws(()=>core.validate(d,'test-hash',rows));
});
test('overlaps and team mismatches are rejected',()=>{
  const d=doc();const c={slot:1,segment:0,start:10,end:11,player_id:'h7'};
  d.corrections=[c,{...c}];assert.throws(()=>core.validate(d,'test-hash',rows));
  d.corrections=[c];d.roster[0].team='away';assert.throws(()=>core.validate(d,'test-hash',rows));
});
test('manual claims still expose collisions symmetrically',()=>{
  const d=doc();d.corrections=[{slot:1,segment:0,start:10,end:12,player_id:'h7'}];
  const r={t:10,players:[agent(1),agent(2)]};
  for(const a of core.preview(r,d)){assert.equal(a.identity.status,'conflict');assert.equal(a.jersey,null);}
});
