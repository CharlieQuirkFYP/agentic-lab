import test from 'node:test';
import assert from 'node:assert/strict';
import { createBenchmarkRunner, DEFAULT_PROTOCOL, expandMatrix, protocolRevision, type BenchmarkSweep, type RunnerDependencies } from '../src/benchmarkRunner.ts';
import { freezeDataset } from '../src/benchmarkDataset.ts';
import type { ObservationKind, PhoneObservation, PhoneObservationContext, StoredPhoneObservation } from '../../shared/phone-benchmarks/types.ts';
const model={id:'asr',name:'ASR',sha256:'a'.repeat(64),bytes:10}, reply={id:'reply',name:'Reasoning',sha256:'b'.repeat(64),bytes:10};

test('protocol identity is stable across property order and separates measurement policies',()=>{
  const p={...DEFAULT_PROTOCOL,sample_resources:true};
  assert.equal(protocolRevision(p),protocolRevision({...p,conditions:{charging:'unrecorded',brightness:'unrecorded',power_mode:'unrecorded'}}));
  assert.notEqual(protocolRevision(p),protocolRevision({...p,sample_resources:false}));
  assert.notEqual(protocolRevision(p),protocolRevision({...p,upload_policy:'live'}));
});
const settings={modelID:model.id,language:'en',dictionary:[],threads:2,maxAudioSeconds:120,metricsEnabled:true};
const configs=[{asr:model,reasoning:reply,settings,reasoning_threads:2}];
const dataset=freezeDataset({schema_version:1,dataset_id:'corpus',version:1,source:'imported',cases:['a','b'].map(id=>({id,title:id,turns:['1','2'].map(turn=>({id:`t${turn}`,uri:`file://${id}${turn}`,sha256:'c'.repeat(64),bytes:32044,duration_ms:1000,sample_rate:16000,channels:1,bits_per_sample:16}))}))});
function harness(overrides: Partial<RunnerDependencies>={}) {
  let id=0,time=0,depth=0,contexts=0;const records:StoredPhoneObservation[]=[];const calls:string[]=[];const saves:BenchmarkSweep[]=[];
  const device={installation_id:'phone',hardware:'fake',os_version:'test',app_version:'test',build:'test'};
  const newID=()=>`id${++id}`;
  const append=(kind:ObservationKind,entity_id:string,context:PhoneObservationContext,data:Record<string,unknown>)=>{const prior=records.filter(x=>x.record.kind===kind&&x.record.entity_id===entity_id).at(-1)?.record;const record:PhoneObservation={schema_version:1,record_id:newID(),producer_stream_id:'stream',record_sequence:records.length+1,timestamp_ms:++time,kind,entity_id,revision:(prior?.revision??0)+1,context:{...prior?.context,...context},data:{...prior?.data,...data}};records.push({device,record,received_at_ms:time});return record;};
  const register=(kind:ObservationKind,context:PhoneObservationContext,data:Record<string,unknown>={})=>{const id=newID();append(kind,id,context,data);return id;};
  const deps:RunnerDependencies={
    controller:{store:{newID,append,registerSweep:data=>register('sweep',{},data),registerJob:(c,d)=>register('job',c,d),registerConversation:(c,d)=>register('conversation',c,d),registerTurn:(c,d)=>register('turn',c,d),registerAttempt:(c,d)=>register('attempt',c,d),snapshot:()=>records},withOwnership:async(_owner,work)=>{assert.equal(depth,0);depth++;calls.push('own');try{return await work();}finally{depth--;calls.push('release');}},runOperation:async(options,work)=>{const record=records.find(x=>x.record.entity_id===options.attemptID)!.record;assert.ok(saves.length);append('attempt',options.attemptID,record.context,{status:'running'});calls.push(`stage:${options.stage}`);try{const result=await work(options.attemptID);append('attempt',options.attemptID,record.context,{status:'succeeded'});return result;}catch(error){append('attempt',options.attemptID,record.context,{status:'failed'});throw error;}},configuration:async()=>register('configuration',{},{}),flush:async()=>{calls.push('flush');},collect:async()=>{calls.push('collect');}},
    native:{listModels:async()=>[model],listReasoningModels:async()=>[reply],configure:async()=>{calls.push('load-asr');},configureReasoning:async()=>{calls.push('load-reply');},transcribeFile:async(uri,attempt)=>{assert.ok(records.some(x=>x.record.entity_id===attempt));assert.equal(depth,1);calls.push(`asr:${uri}:history${contexts}`);return {transcription:{result:{text:`actual ${uri}`,status:'speech'}}};},respond:async(text)=>{calls.push(`reply:${text}:history${contexts}`);contexts++;},resetConversation:async()=>{calls.push('reset');contexts=0;},cancel:async()=>{calls.push('cancel-request');},setBenchmarkActive:async(active)=>{calls.push(`awake:${active}`);},inspect:async()=>({busy:false,closed:false})},
    save:sweep=>{saves.push(structuredClone(sweep));},verify:async()=>{},reserve:()=>{},now:()=>++time,monotonic:()=>++time,delay:async(ms)=>{calls.push(`delay:${ms}`);},...overrides,
  };
  const runner=createBenchmarkRunner(deps);const plan=()=>runner.plan(dataset,configs,{...DEFAULT_PROTOCOL,repetitions:1});
  return {runner,plan,deps,records,calls,saves};
}
test('seeded blocked matrix is complete and repeatable',()=>{const p={...DEFAULT_PROTOCOL,repetitions:2,seed:42};const jobs=expandMatrix(4,['a','b','c'],p);assert.equal(jobs.length,24);assert.deepEqual(jobs,expandMatrix(4,['a','b','c'],p));assert.notDeepEqual(jobs,expandMatrix(4,['a','b','c'],{...p,seed:84}));assert.equal(new Set(jobs.map(j=>`${j.configuration_index}:${j.case_id}:${j.repetition}`)).size,24);assert.throws(()=>expandMatrix(16,['a','b','c'],{...p,repetitions:20}),/240/);});
test('durable matrix precedes native admission, warmup is separate, context persists only within a case',async()=>{const h=harness(),s=h.plan();assert.equal(h.calls.length,0);assert.equal(h.saves[0].jobs.length,2);await h.runner.start(s);assert.equal(s.status,'succeeded');assert.equal(s.jobs.every(j=>j.status==='succeeded'),true);assert.equal(h.records.filter(r=>r.record.kind==='attempt'&&r.record.revision===1&&r.record.data.stage==='warmup').length,2);assert.ok(h.calls.includes('asr:file://a2:history1'));assert.ok(h.calls.includes('asr:file://b1:history0'));assert.deepEqual(h.calls.slice(-3),['reset','awake:false','release']);});
test('pause finishes current conversation and resume never repeats a successful job',async()=>{const h=harness();const original=h.deps.native.respond;h.deps.native.respond=async(...args)=>{const value=await original(...args);if(args[0].includes('file://a1'))h.runner.pause();return value;};const s=h.plan();await h.runner.start(s);assert.equal(s.status,'paused');assert.equal(s.jobs[0].status,'succeeded');assert.equal(s.jobs[1].status,'planned');h.deps.native.respond=original;await h.runner.start(s);assert.equal(s.status,'succeeded');assert.equal(s.jobs[0].executions.length,1);});
test('cancel retains native ownership until the active promise settles and skips remaining jobs',async()=>{const h=harness(),s=h.plan();s.protocol.warmup=false;let finish!:()=>void;let admitted!:()=>void;const admission=new Promise<void>(resolve=>{admitted=resolve;});h.deps.native.transcribeFile=async()=>{admitted();await new Promise<void>(resolve=>{finish=resolve;});return {transcription:{result:{text:'actual recording',status:'speech'}}};};const running=h.runner.start(s);await admission;const cancelling=h.runner.cancel();await Promise.resolve();assert.ok(!h.calls.includes('release'));finish();await cancelling;await running;assert.equal(s.status,'cancelled');assert.equal(s.jobs[0].status,'cancelled');assert.equal(s.jobs[1].status,'skipped');});
test('failure terminates dependent turns and independent cases proceed with clean context',async()=>{const h=harness(),s=h.plan();s.protocol.warmup=false;const original=h.deps.native.transcribeFile;h.deps.native.transcribeFile=async(uri,id)=>{if(uri==='file://a1')throw new Error('native failure');return original(uri,id);};await h.runner.start(s);assert.equal(s.jobs[0].status,'failed');assert.equal(s.jobs[1].status,'succeeded');assert.ok(!h.calls.some(x=>x.startsWith('asr:file://a2')));assert.ok(h.records.some(x=>x.record.kind==='turn'&&x.record.data.status==='skipped'));});
test('process loss retains interrupted execution and resumes from turn one with a new conversation',async()=>{const h=harness(),s=h.plan();s.protocol.warmup=false;s.status='running';s.jobs[0].status='running';const old=h.deps.controller.store.registerConversation({sweep_id:s.id,job_id:s.jobs[0].id},{status:'running'});s.jobs[0].executions.push(old);h.runner.recover(s);assert.equal(s.jobs[0].status,'planned');await h.runner.start(s);assert.equal(s.jobs[0].executions.length,2);assert.notEqual(s.jobs[0].executions[1],old);assert.ok(h.records.some(x=>x.record.entity_id===old&&x.record.data.status==='interrupted'));assert.ok(h.calls.includes('asr:file://a1:history0'));});
test('storage reservation and missing model preflight prevent native inference',async()=>{const h=harness(),s=h.plan();h.deps.native.listModels=async()=>[];await assert.rejects(h.runner.start(s),/unavailable/);assert.equal(s.status,'paused');assert.ok(!h.calls.includes('load-asr'));const bad=harness({save:()=>{throw new Error('disk full');}});assert.throws(()=>bad.plan(),/disk full/);assert.equal(bad.calls.length,0);});
test('background interruption pauses and reruns the same case as a new execution from turn one',async()=>{const h=harness(),s=h.plan();s.protocol.warmup=false;const original=h.deps.native.transcribeFile;let first=true;h.deps.native.transcribeFile=async(uri,id)=>{if(first){first=false;throw Object.assign(new Error('background'),{code:'interrupted'});}return original(uri,id);};await h.runner.start(s);assert.equal(s.status,'paused');assert.equal(s.jobs[0].status,'planned');assert.equal(s.jobs[0].executions.length,1);await h.runner.start(s);assert.equal(s.status,'succeeded');assert.equal(s.jobs[0].executions.length,2);assert.ok(h.calls.includes('asr:file://a1:history0'));});
test('recovery trusts durable terminal observations when crash precedes the plan checkpoint',async()=>{const h=harness(),s=h.plan();s.protocol.warmup=false;await h.runner.start(s);const checkpoint=structuredClone(s);checkpoint.status='running';checkpoint.jobs[0].status='running';h.runner.recover(checkpoint);assert.equal(checkpoint.status,'succeeded');assert.equal(checkpoint.jobs[0].status,'succeeded');});
test('model load failures explicitly finish every blocked job and never invent conversations',async()=>{const h=harness(),s=h.plan();h.deps.native.configure=async()=>{throw new Error('model load failed');};await h.runner.start(s);assert.equal(s.status,'failed');assert.ok(s.jobs.every(job=>job.status==='failed'&&job.reason_code==='configuration_failed'));assert.equal(h.records.filter(item=>item.record.kind==='conversation').length,0);assert.ok(h.records.some(item=>item.record.kind==='attempt'&&item.record.data.status==='failed'));});
test('collection storage loss pauses admission before another stage or independent case',async()=>{const h=harness(),s=h.plan();s.protocol.warmup=false;h.deps.controller.runOperation=async()=>{throw Object.assign(new Error('metrics archive unavailable'),{code:'collection_unavailable'});};await assert.rejects(h.runner.start(s),/metrics archive unavailable/);assert.equal(s.status,'paused');assert.equal(s.jobs[1].status,'planned');assert.ok(!h.calls.some(call=>call.startsWith('asr:')));});
test('recovery cannot terminate an execution that still owns the native runtime',()=>{const h=harness(),s=h.plan();s.status='running';s.jobs[0].status='running';h.deps.controller.getOwner=()=> 'sweep';const before=h.records.length;h.runner.recover(s);assert.equal(s.status,'running');assert.equal(h.records.length,before);});
test('cancel request failure still waits for native settlement before returning',async()=>{const h=harness(),s=h.plan();s.protocol.warmup=false;let finish!:()=>void,admit!:()=>void;const admitted=new Promise<void>(resolve=>{admit=resolve;});h.deps.native.transcribeFile=async()=>{admit();await new Promise<void>(resolve=>{finish=resolve;});return {transcription:{result:{text:'fixture',status:'speech'}}};};h.deps.native.cancel=async()=>{throw new Error('cancel signal failed');};const running=h.runner.start(s);await admitted;let returned=false;const cancel=h.runner.cancel().catch(()=>{returned=true;});await Promise.resolve();assert.equal(returned,false);finish();await cancel;await running;assert.equal(returned,true);assert.equal(s.status,'cancelled');});
test('live mode uploads during stalled inference, prevents overlapping uploads, and removes its timer', async () => {
  let tick!: () => void, stopped = false;
  const h = harness({ scheduleInterval: (callback, interval) => {
    assert.equal(interval, 5000); tick = callback; return () => { stopped = true; };
  } });
  const s = h.plan(); s.protocol.warmup = false; s.protocol.upload_policy = 'live';
  let releaseNative!: () => void, admitted!: () => void, releaseUpload!: () => void;
  const admission = new Promise<void>(resolve => { admitted = resolve; });
  const original = h.deps.native.transcribeFile;
  let first = true, uploads = 0;
  h.deps.native.transcribeFile = async (...args) => {
    if (first) { first = false; admitted(); await new Promise<void>(resolve => { releaseNative = resolve; }); }
    return original(...args);
  };
  h.deps.controller.flush = async () => {
    uploads++;
    if (uploads === 2) await new Promise<void>(resolve => { releaseUpload = resolve; });
  };
  const running = h.runner.start(s); await admission;
  assert.equal(uploads, 1); assert.equal(s.status, 'running');
  tick(); tick(); assert.equal(uploads, 2); assert.equal(stopped, false);
  releaseUpload(); releaseNative(); await running;
  assert.equal(stopped, true); assert.equal(s.status, 'succeeded'); assert.ok(uploads >= 3);
});
test('paused and conversation-boundary protocols never schedule live uploads', async () => {
  for (const policy of ['paused', 'conversation_boundary'] as const) {
    const h = harness({ scheduleInterval: () => { throw new Error('Unexpected live uploader'); } });
    const s = h.plan(); s.protocol.upload_policy = policy;
    await h.runner.start(s);
    assert.equal(h.calls.filter(call => call === 'flush').length, policy === 'paused' ? 0 : 2);
  }
});
test('catalog preflight respects the shared native admission gate', async () => {
  const h = harness(); let busy = false;
  h.deps.native.listModels = async () => {
    busy = true; await Promise.resolve(); busy = false; return [model];
  };
  h.deps.native.listReasoningModels = async () => {
    assert.equal(busy, false, 'Native model readers cannot run concurrently'); return [reply];
  };
  const s = h.plan(); await h.runner.start(s); assert.equal(s.status, 'succeeded');
});
test('native Speech enum is accepted and each model block releases prior runtimes', async () => {
  const h = harness(); const order: string[] = [];
  h.deps.native.close = async () => { order.push('close'); };
  h.deps.native.open = async () => { order.push('open'); };
  h.deps.native.setResourceSamplingEnabled = async enabled => { assert.equal(enabled, false); order.push('sampler'); };
  h.deps.native.configure = async () => { order.push('load'); };
  const transcribe = h.deps.native.transcribeFile;
  h.deps.native.transcribeFile = async (...args) => {
    const result = await transcribe(...args); result.transcription.result.status = 'Speech'; return result;
  };
  const s = h.plan(); s.protocol.sample_resources = false;
  await h.runner.start(s);
  assert.equal(s.status, 'succeeded'); assert.deepEqual(order, ['close','open','sampler','load']);
});
