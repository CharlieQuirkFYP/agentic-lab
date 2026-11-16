import test from 'node:test';
import assert from 'node:assert/strict';
import { DatabaseSync } from 'node:sqlite';
import { createBenchmarkStore } from '../src/benchmarkStore.ts';
import { createBenchmarkCollector } from '../src/benchmarkCollector.ts';
import { createBenchmarkRunner, DEFAULT_PROTOCOL } from '../src/benchmarkRunner.ts';
import { freezeDataset } from '../src/benchmarkDataset.ts';
import { createBenchmarkReport } from '../../shared/phone-benchmarks/index.ts';
import type { MetricsConnection } from '../src/metricsOutbox.ts';
import type { Inspection, LocalReply, LocalResult } from '../modules/pheme-native/src/PhemeNative.types.ts';
const model={id:'asr',name:'ASR fixture',sha256:'a'.repeat(64),bytes:100},reply={id:'reasoning',name:'Reply fixture',sha256:'b'.repeat(64),bytes:100};
const settings={modelID:'asr',language:'en',dictionary:[],threads:2,maxAudioSeconds:120,metricsEnabled:true};
const device={installation_id:'fixture-phone',hardware:'fixture',os_version:'test',app_version:'test',build:'test'};
const snapshot={schema_version:1 as const,state:'ready',ready:true,capabilities:{whisper:true,whisper_metal:false,whisper_coreml:false,metrics_output:true}};
const inspection:Inspection={snapshot,busy:false,closed:false,model,settings,reasoning:null,reasoningModel:reply,runtime_session_id:'fixture-runtime'};
function adapter(db:DatabaseSync):MetricsConnection{return {execSync:sql=>db.exec(sql),runSync:(sql,...args)=>db.prepare(sql).run(...args),getFirstSync:<T>(sql:string,...args:(string|number|null)[])=>db.prepare(sql).get(...args) as T??null,getAllSync:<T>(sql:string,...args:(string|number|null)[])=>db.prepare(sql).all(...args) as T[]};}
test('real archive/controller runner finalizes offline with native-only active durations',async()=>{
 const db=new DatabaseSync(':memory:');let clock=1000,id=0;const store=createBenchmarkStore(adapter(db),{installationID:device.installation_id,uuid:()=>`fixture-${++id}`,now:()=>++clock});
 const controller=createBenchmarkCollector(store,{poll:async()=>({snapshot,observer:{capacity_events:512,pending_events:0,dropped_events:0},batchesJSON:'[]',runtime_session_id:'fixture-runtime'}),inspect:async()=>inspection,device:async()=>device,cancel:async()=>{},settings:()=>({url:'',enabled:false,installationID:device.installation_id}),fetch:async()=>{throw new Error('No server allowed.');},now:()=>++clock,monotonicNow:()=>{clock+=10;return clock;}});
 const native={listModels:async()=>[model],listReasoningModels:async()=>[reply],configure:async()=>snapshot,configureReasoning:async()=>({}),resetConversation:async()=>({}),cancel:async()=>{},setBenchmarkActive:async()=>{},inspect:async()=>inspection,transcribeFile:async(_uri:string,attemptID?:string):Promise<LocalResult>=>({model,settings,execution_location:'device',transcription:{schema_version:1,run_id:attemptID!,result:{text:'local fixture speech',status:'speech',model_id:model.id,processing_time_ms:10}}}),respond:async(_text:string,_request:string,attemptID?:string):Promise<LocalReply>=>({model:reply,execution_location:'device',reply:{run_id:attemptID!,text:'local fixture reply',processing_time_ms:10}})};
 const dataset=freezeDataset({schema_version:1,dataset_id:'fixed',version:1,source:'imported',cases:[{id:'case',title:'Neutral case',turns:[1,2].map(n=>({id:`turn-${n}`,uri:`file://fixture-${n}`,sha256:'c'.repeat(64),bytes:32044,duration_ms:1000,sample_rate:16000,channels:1,bits_per_sample:16}))}]});
 const runner=createBenchmarkRunner({controller,native,save:()=>{},verify:async()=>{},reserve:()=>{},now:()=>++clock,monotonic:()=>++clock,delay:async()=>{}});
 try{await controller.configuration();const sweep=runner.plan(dataset,[{asr:model,reasoning:reply,settings,reasoning_threads:2}],{...DEFAULT_PROTOCOL,repetitions:1,upload_policy:'paused'});await runner.start(sweep);assert.equal(sweep.status,'succeeded');assert.equal(store.isPinned(sweep.id),true);const report=createBenchmarkReport(store.snapshot(),{generated_at_ms:clock,as_of:String(clock),filters:{sweep_id:sweep.id}});assert.equal(report.projection.conversations.length,1);assert.equal(report.projection.conversations[0].duration_ms,40);assert.deepEqual(report.projection.turns.map(t=>t.duration_ms),[20,20]);assert.equal(report.projection.attempts.filter(a=>a.stage==='warmup').length,2);assert.equal(report.projection.sweeps[0].completeness,'complete');assert.ok(store.stats().pending>0);assert.ok(!JSON.stringify(store.snapshot()).includes('local fixture speech'));assert.ok(!JSON.stringify(store.snapshot()).includes('file://fixture'));}finally{db.close();}
});
