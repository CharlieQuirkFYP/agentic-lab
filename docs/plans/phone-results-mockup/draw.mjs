// Local design artifact only. All readings and outcomes below are illustrative.
// Palette follows ios-app/app/metrics.tsx. Frame proportions and safe areas
// adapt the lark-apps creative-design ios-frame.jsx starter.
import { writeFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';

const out = fileURLToPath(new URL('.', import.meta.url));
const c = { ink: '#172033', muted: '#667085', line: '#e4e7ec', green: '#176b5b', light: '#edf7f3', blue: '#718fc5', page: '#f7f8fa', amber: '#ad701d', paleAmber: '#f7e9ce' };
let s = [];
const esc = value => String(value).replaceAll('&', '&amp;').replaceAll('<', '&lt;').replaceAll('>', '&gt;');
const txt = (x,y,value,size=14,color=c.ink,weight=400,anchor='start') => s.push(`<text x="${x}" y="${y}" font-size="${size}" fill="${color}" font-weight="${weight}" text-anchor="${anchor}">${esc(value)}</text>`);
const rect = (x,y,w,h,fill='white',r=12,stroke='none',sw=1) => s.push(`<rect x="${x}" y="${y}" width="${w}" height="${h}" rx="${r}" fill="${fill}" stroke="${stroke}" stroke-width="${sw}"/>`);
const line = (x1,y1,x2,y2,color=c.line,width=1,dash='') => s.push(`<line x1="${x1}" y1="${y1}" x2="${x2}" y2="${y2}" stroke="${color}" stroke-width="${width}" ${dash ? `stroke-dasharray="${dash}"` : ''}/>`);
const path = (d,color=c.green,width=2,fill='none') => s.push(`<path d="${d}" fill="${fill}" stroke="${color}" stroke-width="${width}" stroke-linecap="round" stroke-linejoin="round"/>`);
const circle = (x,y,r,fill) => s.push(`<circle cx="${x}" cy="${y}" r="${r}" fill="${fill}"/>`);
const chevron = (x,y,color=c.muted) => path(`M ${x} ${y-5} l 5 5 l -5 5`,color,1.6);
const back = (label) => {path('M 28 75 l -6 6 l 6 6',c.green,1.8);txt(36,86,label,15,c.green,500);};
const card = (y,h,border=c.line) => rect(20,y,362,h,'white',12,border);
const pill = (x,y,w,label,fill=c.light,color=c.green) => {rect(x,y,w,26,fill,13);txt(x+w/2,y+18,label,12,color,600,'middle');};
const sectionLabel = (x,n,title,subtitle) => {circle(x+14,204,14,c.green);txt(x+14,209,n,13,'white',700,'middle');txt(x+40,209,title,21,c.ink,650);txt(x,237,subtitle,14,c.muted);};
const phone = (x,label) => {
  s.push(`<g transform="translate(${x},260)" data-screen-label="${esc(label)}">`);
  rect(0,0,402,874,c.page,48,'#cbd2da',1.4);
  rect(138,11,126,37,c.ink,24);
  txt(34,36,'9:41',16,c.ink,650);
  [6,9,12,15].forEach((h,i)=>rect(302+i*5,35-h,3,h,c.ink,1));
  path('M 327 26 Q 335 19 343 26 M 330 29 Q 335 25 340 29',c.ink,1.7);circle(335,33,1.4,c.ink);
  rect(350,22,23,12,'none',3,c.ink);rect(352,24,17,8,c.ink,1);rect(374,26,2,4,c.ink,1);
};
const endPhone = () => {rect(132,858,138,5,'#a9adb5',3);s.push('</g>');};

const segments = [
  {start:0,values:[2.40,2.43,2.55,2.64,2.76,2.85,2.96,3.04,3.07,3.09,3.08,3.10,3.08,3.06,3.04,3.02]},
  {start:25,values:[2.76,2.81,2.88,2.96,3.04,3.07,3.10,3.14,3.12,3.16,3.18,3.14,3.16,3.12,3.11,3.09,3.07,3.05,3.02]},
  {start:55,values:[2.80,2.84,2.90,2.96,3.00,3.04,3.07,3.10,3.12,3.11,3.09,3.07,3.08,3.06,3.04,3.02,3.00]}
];
const readings = segments.flatMap((seg,i)=>seg.values.map((value,j)=>({t:seg.start+j*0.5,value,turn:i+1})));
const opReadings = readings.filter(r=>r.turn===2&&r.t>=27);
const stats = rows => ({latest:rows.at(-1).value,min:Math.min(...rows.map(r=>r.value)),max:Math.max(...rows.map(r=>r.value)),avg:rows.reduce((a,r)=>a+r.value,0)/rows.length,n:rows.length});
const overall = stats(readings), operation=stats(opReadings);
function plot(x,y,w,h,{detail=false,operationOnly=false}={}) {
  const min=2.3,max=3.3,tMin=operationOnly?26.8:0,tMax=operationOnly?34.5:64;
  const px=t=>x+(t-tMin)/(tMax-tMin)*w, py=v=>y+h-(v-min)/(max-min)*h;
  [2.4,2.8,3.2].forEach(v=>{line(x,py(v),x+w,py(v));if(detail)txt(x-9,py(v)+4,v.toFixed(1),11,c.muted,400,'end');});
  const groups = operationOnly?[opReadings]:segments.map((_,i)=>readings.filter(r=>r.turn===i+1));
  if(!operationOnly) {
    [[7.5,25],[34,55]].forEach(([a,b])=>rect(px(a),y,px(b)-px(a),h,'url(#gaps)',0));
  }
  groups.forEach(group=>{
    const d=group.map((r,i)=>`${i?'L':'M'} ${px(r.t).toFixed(2)} ${py(r.value).toFixed(2)}`).join(' ');
    path(`${d} L ${px(group.at(-1).t)} ${y+h} L ${px(group[0].t)} ${y+h} Z`,'none',0,'#e5f0ec');
    path(d,c.green,2.4);
  });
  (operationOnly?[27,30,34]:[0,20,40,64]).forEach(t=>txt(px(t),y+h+19,`${t}s`,11,c.muted,400,'middle'));
  if(detail) {
    const peak=readings.find(r=>r.value===3.18);circle(px(peak.t),py(peak.value),4,c.green);
    pill(px(peak.t)-37,py(peak.value)-33,74,'3.18 GiB');
  }
}
function statGrid(y,values) {
  card(y,118);line(201,y+14,201,y+104);line(34,y+59,368,y+59);
  values.forEach(([label,value],i)=>{const x=i%2?218:36, yy=y+21+Math.floor(i/2)*58;txt(x,yy,label,12,c.muted);txt(x,yy+25,`${value.toFixed(2)} GiB`,21,c.ink,600);});
}

s.push(`<svg xmlns="http://www.w3.org/2000/svg" width="1460" height="1294" viewBox="0 0 1460 1294" role="img" aria-labelledby="title desc"><title id="title">Pheme phone Results visual draft</title><desc id="desc">Three proposed iPhone screens: overall conversation metrics and graphs, the full app-memory metric graph, and app memory during Turn 2 reply generation. All data is illustrative.</desc><defs><pattern id="gaps" width="7" height="7" patternUnits="userSpaceOnUse"><rect width="7" height="7" fill="#f8f9fb"/><path d="M 0 7 L 7 0" stroke="#e9ecf0" stroke-width="1"/></pattern></defs><g font-family="-apple-system, BlinkMacSystemFont, 'Helvetica Neue', sans-serif" style="font-variant-numeric:tabular-nums">`);
rect(0,0,1460,1294,'#eef1f4',0);
txt(58,48,'PHEME / PHONE RESULTS',14,c.green,700);
txt(58,99,'One conversation. Every metric. The detail behind it.',34,c.ink,650);
txt(58,132,'Overall graphs → individual metric → turn / operation',19,c.muted);
pill(1138,31,264,'VISUAL DRAFT · EXAMPLE DATA','#fff',c.muted);
line(58,163,1402,163,'#d6dce3');
sectionLabel(58,'1','Overall results','See the whole conversation at a glance.');
sectionLabel(529,'2','One metric','Tap App memory to see its full history.');
sectionLabel(1000,'3','One operation','Narrow the graph to a single reply.');
path('M 465 203 H 503 m -6 -5 l 6 5 l -6 5','#9caab7',1.6);
path('M 937 203 H 974 m -6 -5 l 6 5 l -6 5','#9caab7',1.6);

// Screen 1: all turns, summary + overall graphs.
phone(58,'Overall results');back('Chat');txt(20,120,'Results',29,c.ink,700);txt(380,119,'Share',15,c.green,600,'end');
txt(20,143,'Saved on this phone',12,c.muted);circle(147,139,3,c.green);
card(155,54);txt(36,178,'Latest conversation',15,c.ink,600);txt(36,197,'13 Nov, 09:41 · Whisper small + Qwen3',12,c.muted);chevron(359,181);
card(221,93);line(186,235,186,300);txt(36,243,'Completed turns',12,c.muted);txt(36,274,'3 / 3',28,c.ink,650);txt(36,298,'0 failed · 0 cancelled',11,c.muted);
txt(202,243,'Recorded processing',12,c.muted);txt(202,274,'25.9 s',28,c.ink,650);txt(202,298,'6 operations',11,c.muted);
card(326,196);txt(36,352,'Processing by turn',16,c.ink,600);txt(362,352,'seconds',11,c.muted,400,'end');
circle(40,373,4,c.blue);txt(50,377,'Speech processing',11,c.muted);circle(192,373,4,c.green);txt(202,377,'Reply generation',11,c.muted);
const turns=[{asr:1.4,reply:6.5},{asr:1.8,reply:7.7},{asr:1.6,reply:6.9}];
const bx=96,bw=220;
[0,5,10].forEach(v=>{const x=bx+v*bw/10;line(x,398,x,495,'#eef1f4');txt(x,405,v,10,c.muted,400,'middle');});
turns.forEach((t,i)=>{const y=419+i*29;txt(36,y+12,`Turn ${i+1}`,12,c.ink);rect(bx,y,t.asr*bw/10,14,c.blue,3);rect(bx+t.asr*bw/10,y,t.reply*bw/10,14,c.green,3);txt(362,y+12,(t.asr+t.reply).toFixed(1),12,c.ink,600,'end');});
txt(36,508,'Tap a stage to inspect its operation',11,c.muted);
card(534,143,c.green);txt(36,559,'App memory',16,c.ink,600);txt(348,559,'3.18 GiB peak',12,c.green,600,'end');chevron(361,555,c.green);
plot(60,575,286,60);txt(36,666,'Whole app · 52 readings · tap for graph',11,c.muted);
card(689,106);txt(36,714,'Thermal state',16,c.ink,600);chevron(361,710);circle(41,733,4,c.green);txt(51,737,'Nominal',11,c.muted);circle(133,733,4,c.amber);txt(143,737,'Fair',11,c.muted);txt(362,737,'52 readings',10,c.muted,400,'end');
rect(60,749,286,15,'url(#gaps)',3);
[[0,7.5,c.green],[25,34,c.amber],[55,63,c.amber]].forEach(([a,b,color])=>rect(60+a/64*286,749,(b-a)/64*286,15,color,2));
[0,20,40,64].forEach(t=>txt(60+t/64*286,783,`${t}s`,10,c.muted,400,'middle'));
rect(20,805,362,44,'#edf7f3',10);txt(36,823,'All metrics',13,c.green,600);txt(36,840,'CPU, battery, stage timings and more',11,c.muted);chevron(360,827,c.green);endPhone();

// Screen 2: full conversation memory and TUI-like numeric statistics.
phone(529,'App memory across conversation');back('Results');txt(20,122,'App memory',28,c.ink,700);txt(20,146,'Latest conversation · 13 Nov, 09:41',12,c.muted);
rect(20,166,362,46,'#e9edf1',10);rect(24,170,174,38,'white',8);txt(111,194,'All turns',14,c.green,650,'middle');txt(287,194,'Choose a turn',14,c.muted,500,'middle');
card(228,278);txt(36,253,'Whole conversation',15,c.ink,600);txt(362,253,'GiB',12,c.muted,400,'end');
plot(60,297,286,140,{detail:true});txt(203,474,'Elapsed from conversation start',11,c.muted,400,'middle');rect(36,487,12,9,'url(#gaps)',1);txt(55,496,'Gaps = no recorded readings',11,c.muted);
statGrid(518,[['Latest',overall.latest],['Sampled peak',overall.max],['Minimum',overall.min],['Average',overall.avg]]);
txt(20,656,'52 readings · whole-app memory',12,c.muted);
card(674,132);txt(36,700,'Inspect a turn',15,c.ink,600);
txt(36,732,'Turn 1',14,c.ink,500);txt(345,732,'2 operations',12,c.muted,400,'end');chevron(359,727);line(36,745,366,745);
rect(28,751,346,47,c.light,8);txt(40,780,'Turn 2',14,c.green,650);txt(345,780,'2 operations',12,c.green,400,'end');chevron(359,775,c.green);
txt(20,833,'View readings and measurement details',12,c.green,500);chevron(361,829,c.green);endPhone();

// Screen 3: selected memory metric retained while narrowing to a reply.
phone(1000,'App memory during Turn 2 reply');back('App memory');txt(20,122,'Turn 2',28,c.ink,700);txt(20,146,'Latest conversation · 13 Nov, 09:41',12,c.muted);
rect(20,166,362,46,'#e9edf1',10);txt(111,194,'Speech processing',12,c.muted,500,'middle');rect(203,170,175,38,'white',8);txt(290,194,'Reply generation',13,c.green,650,'middle');
card(228,80);txt(36,253,'Reply generation',15,c.ink,600);pill(277,237,88,'Completed');txt(36,284,'7.7 s',24,c.ink,650);txt(127,282,'Qwen3 · attempt 1',12,c.muted);
card(322,258);txt(36,347,'App memory',16,c.ink,600);txt(362,347,'GiB',12,c.muted,400,'end');
plot(60,384,286,116,{detail:true,operationOnly:true});txt(203,536,'Elapsed from conversation start',11,c.muted,400,'middle');txt(36,562,'15 readings during this operation',12,c.muted);
statGrid(592,[['Latest',operation.latest],['Sampled peak',operation.max],['Minimum',operation.min],['Average',operation.avg]]);
txt(20,734,'Memory belongs to the whole app,',12,c.muted);txt(20,752,'sampled while this reply was generated.',12,c.muted);
rect(20,768,362,48,'white',10,c.line);txt(36,798,'View the 15 readings',14,c.green,600);chevron(359,792,c.green);txt(201,840,'Back to all turns',13,c.green,600,'middle');endPhone();

txt(58,1170,'Overview stays visual.',17,c.ink,650);txt(58,1195,'Totals and graphs cover all turns in the chat.',14,c.muted);
txt(529,1170,'Every metric gets its own graph.',17,c.ink,650);txt(529,1195,'Units, readings and statistics stay together.',14,c.muted);
txt(1000,1170,'Keep the context as you drill down.',17,c.ink,650);txt(1000,1195,'Same chat, same metric, one operation.',14,c.muted);
line(58,1222,1402,1222,'#d6dce3');txt(58,1254,'Proposed screens · illustrative values, not benchmark measurements · content scrolls on phone',13,c.muted);
txt(1402,1254,'Agentic Lab / 13 Nov 2026',13,c.muted,400,'end');
s.push('</g></svg>');
const svg=s.join('\n');
writeFileSync(out+'phone-results-draft.svg',svg+'\n');
writeFileSync(out+'index.html',`<!doctype html>\n<html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><title>Pheme phone Results — visual draft</title><style>body{margin:0;background:#eef1f4}main{max-width:1460px;margin:auto}svg{display:block;width:100%;height:auto}a{color:#176b5b}footer{font:14px -apple-system,system-ui;padding:18px 4%;display:flex;gap:24px}</style></head><body><main>${svg}</main><footer><a href="phone-results-draft.png">Full-size PNG</a><a href="phone-results-draft.svg">Scalable SVG</a><a href="../phone-results-graphs-plan.md">Implementation plan</a></footer></body></html>\n`);
console.log(JSON.stringify({files:['phone-results-draft.svg','index.html'],overall,operation,recorded_processing_seconds:turns.reduce((a,t)=>a+t.asr+t.reply,0)},null,2));
