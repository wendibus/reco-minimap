// UI-Rauchtest: laedt http://127.0.0.1:18790/ in jsdom und prueft alle Schritte in allen Sprachen. Siehe README.
// Voraussetzung: `HOME=<tmp> reco-minimap ui --no-open --port 18790` laeuft mit einem Projekt (Modell importiert, court.json, teams.json) im Standard-Projektordner; `npm i jsdom`.
const {JSDOM}=require('jsdom');
const sleep=ms=>new Promise(r=>setTimeout(r,ms));
(async()=>{
  const dom=await JSDOM.fromURL('http://127.0.0.1:18790/',{runScripts:'dangerously',resources:'usable',pretendToBeVisual:true,
    beforeParse(w){w.fetch=(u,o)=>fetch(new URL(u,'http://127.0.0.1:18790/').href,o);}});
  const w=dom.window; await sleep(1500);
  const $=id=>w.document.getElementById(id);
  const fail=m=>{console.error('FAIL:',m);process.exit(1);};
  if(!$('nav')||$('nav').querySelectorAll('button').length!==6) fail('nav');
  const errors=[]; w.addEventListener('error',e=>errors.push(e.message));
  const steps=['setup','video','calib','run','live','eval'];
  const out={};
  for(const lang of ['de','en','fr','es']){
    $('lang').value=lang; $('lang').dispatchEvent(new w.Event('change')); await sleep(150);
    for(const s of steps){
      [...$('nav').querySelectorAll('button')].find(b=>b.dataset.s===s).click(); await sleep(120);
      const html=$('panel').innerHTML;
      if(html.length<80) fail(lang+' '+s+' empty panel');
      if(/\b(sport|model|video|cal|run|live|eval|step|btn|status|zone1|lm)\.[a-z0-9_]+\b/.test($('panel').textContent+$('nav').textContent)) fail(lang+' '+s+' untranslated key: '+($('panel').textContent+$('nav').textContent).match(/\b(sport|model|video|cal|run|live|eval|step|btn|status|zone1|lm)\.[a-z0-9_]+\b/)[0]);
    }
    out[lang]=$('nav').textContent.replace(/\s+/g,' ').trim();
  }
  console.log(JSON.stringify(out,null,1));
  // sport switching + custom fields
  [...$('nav').querySelectorAll('button')].find(b=>b.dataset.s==='setup').click(); await sleep(150);
  $('lang').value='fr'; $('lang').dispatchEvent(new w.Event('change')); await sleep(150);
  const opts=[...$('sportsel').options].map(o=>o.textContent); console.log('Sportarten ('+opts.length+', fr):',opts.join(' | ')); if(opts.length!==10) fail('expected 10 sports');
  $('sportsel').value='football'; $('sportsel').dispatchEvent(new w.Event('change')); await sleep(150);
  console.log('Football-Info:', $('panel').textContent.match(/105 × 68[^\n]*/)?.[0]?.slice(0,120));
  console.log('Modell-Warnung bei Sport-Konflikt:', /entraîné|«/.test($('panel').textContent)?'ja':'nein');
  $('sportsel').value='custom'; $('sportsel').dispatchEvent(new w.Event('change')); await sleep(150);
  console.log('Custom-Felder sichtbar:', !!w.document.querySelector('[data-k="custom.length"]'));
  // import button with an empty path gives a clear message, not ': nicht gefunden'
  $('sportsel').value='basketball'; $('sportsel').dispatchEvent(new w.Event('change')); await sleep(150);
  $('lang').value='de'; $('lang').dispatchEvent(new w.Event('change')); await sleep(150);
  $('mpath').value=''; $('mimport').click(); await sleep(200);
    if(!$('mtest')||$('mtest').disabled) fail('model check button missing or disabled');
  console.log('Modelltest-Knopf: ok');
  if(!$('log').textContent.includes('Bitte zuerst')) fail('empty path message');
  if(errors.length) fail('JS errors: '+errors.join(' / '));
  console.log('UI-Test ok'); process.exit(0);
})().catch(e=>{console.error('ERR',e);process.exit(1);});
