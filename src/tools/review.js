/* Shared, testable review logic. Coordinates stay in field metres: resizing the
 * view never changes calibration or saved corrections. No external libraries. */
'use strict';
const ReviewCore = (() => {
  function frames(text) {
    const rows = text.trim().split(/\r?\n/).map((line, i) => {
      const r = JSON.parse(line);
      if (!Number.isFinite(r.t) || r.t < 0 || !Array.isArray(r.players) || r.identity_review) throw Error(`Ungültiger oder bereits korrigierter Frame ${i + 1}`);
      if (r.events != null && (!Array.isArray(r.events) || r.events.some(e => !e || typeof e !== 'object'))) throw Error(`Ungültige Ereignisse in Frame ${i + 1}`);
      for (const a of r.players) {
        if (![a.id, a.segment].every(n => Number.isSafeInteger(n) && n >= 0) || ![a.x, a.y].every(Number.isFinite)) throw Error(`Tracking-Abschnitt/Position fehlt in Frame ${i + 1}`);
      }
      return r;
    });
    if (rows.length > 100000) throw Error('Maximal 100.000 Frames pro Review; Analyseabschnitt verkürzen.');
    if (rows.some((r,i) => i && r.t < rows[i-1].t)) throw Error('Analysezeiten sind nicht sortiert.');
    return rows;
  }
  // Never use a future observation, nor bridge an arbitrarily long gap.
  function at(rows, t, maxGap = 1) {
    let lo = 0, hi = rows.length;
    while (lo < hi) { const m = (lo + hi) >> 1; if (rows[m].t <= t) lo = m + 1; else hi = m; }
    const r = rows[lo - 1];
    return r && t - r.t <= maxGap ? r : null;
  }
  function validate(doc, hash, rows) {
    if (doc.schema_version !== 1 || doc.source_sha256 !== hash || !Array.isArray(doc.roster) || !Array.isArray(doc.corrections)) throw Error('Review passt nicht zur Originalanalyse oder zum Schema.');
    const ids = new Set(), numbers = new Set();
    for (const p of doc.roster) {
      const key = `${p.team}:${p.number}`;
      if (typeof p.id !== 'string' || !p.id.trim() || typeof p.name !== 'string' || !p.name.trim() || !['home','away'].includes(p.team) || !Number.isInteger(p.number) || p.number < 0 || p.number > 99 || ids.has(p.id) || numbers.has(key)) throw Error('Ungültiger oder doppelter Kadereintrag.');
      ids.add(p.id); numbers.add(key);
    }
    doc.corrections.forEach((c, i) => {
      if (![c.slot,c.segment].every(n => Number.isSafeInteger(n) && n >= 0) || ![c.start,c.end].every(Number.isFinite) || c.start < 0 || c.end <= c.start || (c.player_id !== null && !ids.has(c.player_id))) throw Error('Ungültige Zuordnung.');
      if (doc.corrections.slice(0,i).some(q => q.slot === c.slot && q.segment === c.segment && q.start < c.end && c.start < q.end)) throw Error('Zeiträume desselben Tracking-Abschnitts überschneiden sich.');
      let found = false;
      for (const r of rows) if (r.t >= c.start && r.t < c.end) for (const a of r.players) if (a.id === c.slot && a.segment === c.segment) {
        found = true;
        const p = doc.roster.find(p => p.id === c.player_id);
        if (p && p.team !== a.team) throw Error('Kader und erkannte Mannschaft stimmen nicht überein.');
      }
      if (!found) throw Error('Zuordnung trifft keinen Frame/Tracking-Abschnitt.');
    });
    return doc;
  }
  function preview(frame, doc) {
    const players = frame.players.map(raw => {
      const a = structuredClone(raw);
      const c = doc.corrections.find(c => c.slot === a.id && c.segment === a.segment && c.start <= frame.t && frame.t < c.end);
      if (c) {
        const p = doc.roster.find(p => p.id === c.player_id);
        a.identity = p ? {status:'manual',player:{team:p.team,number:p.number},name:p.name} : {status:'unknown',player:null};
        a.jersey = p ? p.number : null;
      }
      return a;
    });
    for (const a of players) {
      const p = a.identity?.player;
      if (p && a.state !== 'lost' && players.filter(b => b.state !== 'lost' && b.identity?.player?.team === p.team && b.identity?.player?.number === p.number).length > 1) { a.identity.status = 'conflict'; a.jersey = null; }
    }
    return players;
  }
  return {frames, at, validate, preview};
})();
if (typeof module !== 'undefined') module.exports = ReviewCore;
if (typeof document !== 'undefined') (async function boot() {
  const $ = id => document.getElementById(id), video = $('video'), canvas = $('map'), ctx = canvas.getContext('2d');
  let rows = [], hash = '', doc = {schema_version:1,source_sha256:'',roster:[],corrections:[]}, selected = null, hits = [], videoUrl = '', dirty = false, sports = [], loadVersion = 0;
  const status = message => { $('status').textContent = message; };
  const guard = fn => (...args) => { try { fn(...args); } catch(e) { status(e.message); } };
  const offset = () => Number($('offset').value) || 0;
  function download() {
    ReviewCore.validate(doc, hash, rows);
    const text=JSON.stringify(doc,null,2);
    $('exportFallback').hidden=false;$('exportText').value=text;
    const url = URL.createObjectURL(new Blob([text],{type:'application/json'}));
    const a = document.createElement('a'); a.href = url; a.download = 'review.json'; document.body.append(a); a.click(); a.remove();
    setTimeout(() => URL.revokeObjectURL(url),1000);
    status('Download angefordert. Falls blockiert: Exporttext kopieren und als review.json speichern.');
  }
  function controls() {
    $('rosterList').textContent = doc.roster.map(p => `${p.team} · #${p.number} · ${p.name}`).join('\n');
    const previousPlayer=$('player').value;
    $('player').replaceChildren(new Option('Unbekannt',''));
    for (const p of doc.roster) $('player').add(new Option(`${p.team} #${p.number} · ${p.name}`,p.id));
    $('player').value=previousPlayer;
    $('corrections').replaceChildren();
    doc.corrections.forEach((c,i) => {
      const line = document.createElement('p'), remove = document.createElement('button');
      line.append(document.createTextNode(`Slot ${c.slot}/${c.segment} · ${c.start}–${c.end}s → ${c.player_id ?? 'unbekannt'} `));
      remove.textContent = 'Entfernen'; remove.onclick = () => {doc.corrections.splice(i,1);dirty=true;controls();};
      line.append(remove); $('corrections').append(line);
    });
    $('export').disabled = !hash;
    $('addPlayer').disabled = !hash;
  }
  function events() {
    const all = rows.flatMap(r => (r.events || []).map(e => ({...e,t:Number.isFinite(e.t)?e.t:r.t}))), seen = new Set();
    const unique = all.filter(e => {const k=JSON.stringify(e);if(seen.has(k))return false;seen.add(k);return true;});
    const type = e => String(e.type || e.kind || 'Ereignis');
    const previous = $('eventType').value;
    $('eventType').replaceChildren(new Option('Alle',''));
    for(const t of new Set(unique.map(type))) $('eventType').add(new Option(t,t));
    $('eventType').value = previous;
    const render = () => {
      $('events').replaceChildren();
      const matching=unique.filter(e=>!$('eventType').value || type(e)===$('eventType').value);
      for(const e of matching.slice(0,500)) {
        const b=document.createElement('button');b.textContent=`${e.t.toFixed(2)} s · ${type(e)} · ${e.kind || ''}`;
        b.onclick=()=>{if(Number.isFinite(video.duration)){video.currentTime=Math.max(0,Math.min(video.duration,e.t-offset()-2));video.pause();}};
        $('events').append(b);
      }
      const n=document.createElement('p');n.textContent=`${matching.length} Ereignisse, ${Math.min(500,matching.length)} angezeigt.`;$('events').append(n);
    };
    $('eventType').onchange=render;render();
  }
  $('analysisFile').onchange = async () => {
    const file=$('analysisFile').files[0];if(!file)return;
    if(dirty && !confirm('Nicht heruntergeladene Zuordnungen verwerfen?'))return;
    const version=++loadVersion;
    rows=[];hash='';doc={schema_version:1,source_sha256:'',roster:[],corrections:[]};dirty=false;selected=null;$('assign').disabled=true;$('export').disabled=true;$('selected').textContent='Noch kein Spieler ausgewählt.';$('events').replaceChildren();$('exportFallback').hidden=true;controls();
    try {
      if(file.size>64*1024*1024)throw Error('Analyse größer als 64 MiB. Bitte einen kürzeren Abschnitt verwenden.');
      const bytes=await file.arrayBuffer();const parsed=ReviewCore.frames(new TextDecoder().decode(bytes));
      const digest=Array.from(new Uint8Array(await crypto.subtle.digest('SHA-256',bytes)),n=>n.toString(16).padStart(2,'0')).join('');
      if(version!==loadVersion)return;
      rows=parsed;hash=digest;doc={schema_version:1,source_sha256:hash,roster:[],corrections:[]};dirty=false;
      controls();events();status(`${rows.length} Frames geladen. Sport und Zeitversatz prüfen.`);
    }catch(e){if(version===loadVersion)status(e.message);}
  };
  $('videoFile').onchange=()=>{const f=$('videoFile').files[0];if(!f)return;if(videoUrl)URL.revokeObjectURL(videoUrl);videoUrl=URL.createObjectURL(f);video.src=videoUrl;};
  video.onloadedmetadata=()=>{$('seek').max=video.duration;$('seek').disabled=!Number.isFinite(video.duration);};
  video.onerror=()=>status('Video kann vom Browser nicht abgespielt werden. Unterstützten Codec verwenden.');
  $('seek').oninput=()=>{video.currentTime=Number($('seek').value);};
  $('addPlayer').onclick=guard(()=>{
    const number=Number($('number').value), team=$('team').value, name=$('name').value.trim();
    if(!$('number').value.trim())throw Error('Nummer fehlt.');
    const next=structuredClone(doc);next.roster.push({id:`${team}-${number}`,team,number,name});ReviewCore.validate(next,hash,rows);doc=next;dirty=true;controls();
  });
  $('assign').onclick=guard(()=>{
    if(!selected)throw Error('Spieler auf der Karte auswählen.');
    if(!$('start').value || !$('end').value)throw Error('Zeitraum fehlt.');
    const next=structuredClone(doc);next.corrections.push({slot:selected.id,segment:selected.segment,start:Number($('start').value),end:Number($('end').value),player_id:$('player').value||null});
    ReviewCore.validate(next,hash,rows);doc=next;dirty=true;controls();status('Zuordnung als Vorschau übernommen. Zum Speichern review.json herunterladen.');
  });
  $('export').onclick=guard(download);
  $('reviewFile').onchange=async()=>{try{
    if(!hash)throw Error('Zuerst Originalanalyse laden.');
    const f=$('reviewFile').files[0];if(!f)return;if(f.size>1024*1024)throw Error('Review-Datei größer als 1 MiB.');
    if(dirty&&!confirm('Nicht heruntergeladene Änderungen ersetzen?'))return;
    const next=JSON.parse(await f.text());ReviewCore.validate(next,hash,rows);doc=next;dirty=false;controls();status('Review geladen.');
  }catch(e){status(e.message);}};
  canvas.onclick=e=>{
    const rect=canvas.getBoundingClientRect(), x=(e.clientX-rect.left)*canvas.width/rect.width,y=(e.clientY-rect.top)*canvas.height/rect.height;
    const hit=hits.map(h=>({...h,d:Math.hypot(h.x-x,h.y-y)})).sort((a,b)=>a.d-b.d)[0];if(!hit||hit.d>hit.r*2)return;
    video.pause();selected={id:hit.a.id,segment:hit.a.segment};
    $('selected').textContent=`Slot ${selected.id} · Abschnitt ${selected.segment} · ${hit.a.team} · ${hit.a.identity?.status || 'unknown'}`;
    const t=video.currentTime+offset();$('start').value=Math.max(0,hit.t).toFixed(2);$('end').value=(Math.max(t,hit.t)+5).toFixed(2);$('assign').disabled=false;
  };
  function draw() {
    const dpr=window.devicePixelRatio||1,w=Math.round(canvas.clientWidth*dpr),h=Math.round(canvas.clientHeight*dpr);
    if(canvas.width!==w||canvas.height!==h){canvas.width=w;canvas.height=h;}
    ctx.clearRect(0,0,w,h);hits=[];
    const sport=sports.find(s=>s.id===$('sport').value), t=video.currentTime+offset();
    $('seek').value=video.currentTime;$('clock').textContent=`Video ${video.currentTime.toFixed(2)} s · Analyse ${t.toFixed(2)} s`;
    if(sport){
      const [l,b]=sport.field,scale=Math.min((w-30*dpr)/l,(h-30*dpr)/b),ox=(w-l*scale)/2,oy=(h-b*scale)/2;
      ctx.strokeStyle='#ede3d1';ctx.lineWidth=1.5*dpr;
      for(const line of sport.lines){ctx.beginPath();line.forEach(([x,y],i)=>i?ctx.lineTo(ox+x*scale,oy+y*scale):ctx.moveTo(ox+x*scale,oy+y*scale));ctx.stroke();}
      const frame=ReviewCore.at(rows,t);
      if(frame){
        for(const a of ReviewCore.preview(frame,doc)){
          const x=ox+a.x*scale,y=oy+a.y*scale,r=Math.max(6*dpr,scale*0.35);
          ctx.globalAlpha=a.state==='lost'?0.3:1;ctx.fillStyle=a.identity?.status==='conflict'?'#ff5a5a':a.team==='home'?'#fff':'#4488ff';ctx.beginPath();ctx.arc(x,y,r,0,2*Math.PI);ctx.fill();ctx.globalAlpha=1;
          if(selected?.id===a.id&&selected?.segment===a.segment){ctx.strokeStyle='#ff8c00';ctx.lineWidth=2*dpr;ctx.stroke();}
          ctx.fillStyle='#ffd28b';ctx.font=`${12*dpr}px system-ui`;ctx.fillText(a.jersey==null?`? ${a.id}`:`#${a.jersey}`,x+r+2*dpr,y);
          hits.push({x,y,r,a,t:frame.t});
        }
        const ball=frame.ball;if(ball&&Number.isFinite(ball.x)&&Number.isFinite(ball.y)&&ball.state!=='unknown'){ctx.fillStyle='#ff8c00';ctx.beginPath();ctx.arc(ox+ball.x*scale,oy+ball.y*scale,4*dpr,0,2*Math.PI);ctx.fill();}
      }else{ctx.fillStyle='#ffd28b';ctx.font=`${14*dpr}px system-ui`;ctx.fillText('Keine zeitnahen Analysedaten',20*dpr,25*dpr);}
    }
    requestAnimationFrame(draw);
  }
  window.addEventListener('beforeunload',e=>{if(dirty){e.preventDefault();e.returnValue='';}});
  try {const r=await fetch('/api/review/sports');if(!r.ok)throw Error('Sportprofile nicht verfügbar');sports=(await r.json()).sports.filter(s=>s.id!=='custom');for(const s of sports)$('sport').add(new Option(s.id,s.id));$('sport').value='basketball';}catch(e){status(e.message);}
  controls();requestAnimationFrame(draw);
})();
