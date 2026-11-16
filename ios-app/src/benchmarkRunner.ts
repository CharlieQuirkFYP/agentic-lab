import { datasetMetadata, freezeDataset, sha256, type BenchmarkDataset } from './benchmarkDataset.ts';
import type { Model, Settings } from '../modules/pheme-native/src/PhemeNative.types.ts';
import type { AttemptStage, ExecutionStatus, ObservationKind, PhoneObservation, PhoneObservationContext, StoredPhoneObservation } from '../../shared/phone-benchmarks/types.ts';

export interface BenchmarkConfiguration { asr: Model; reasoning: Model | null; settings: Settings; reasoning_threads: number }
export interface BenchmarkProtocol {
  sample_resources?: boolean;
  version: 1; automatic_retries?: 0; mode: 'fixed_script' | 'asr_only'; repetitions: number; seed: number; cooldown_ms: number;
  warmup: boolean; upload_policy: 'conversation_boundary' | 'paused' | 'live';
  thermal_policy: 'fixed_interval'; conditions: { brightness: string; power_mode: string; charging: string };
}
export interface BenchmarkJob {
  id: string; configuration_index: number; case_id: string; repetition: number; block: number;
  status: ExecutionStatus; executions: string[]; configuration_id?: string; reason_code?: string;
}
export interface BenchmarkSweep {
  schema_version: 1; id: string; source: 'sweep' | 'acceptance'; created_at_ms: number;
  status: 'planned' | 'running' | 'paused' | 'succeeded' | 'failed' | 'cancelled';
  dataset: BenchmarkDataset; configurations: BenchmarkConfiguration[]; protocol: BenchmarkProtocol; protocol_revision?: string;
  jobs: BenchmarkJob[]; current_job_id?: string; current_stage?: string; acceptance_id?: string;
}
export interface RunnerStore {
  assertCanStart?(): void;
  finalize?(context: PhoneObservationContext, data: Record<string, unknown>): PhoneObservation;
  newID(): string; pin?(id: string, pinned: boolean): void;
  append(kind: ObservationKind, id: string, context: PhoneObservationContext, data: Record<string, unknown>): PhoneObservation;
  registerSweep(data: Record<string, unknown>): string;
  registerJob(context: PhoneObservationContext, data: Record<string, unknown>): string;
  registerConversation(context: PhoneObservationContext, data?: Record<string, unknown>): string;
  registerTurn(context: PhoneObservationContext, data?: Record<string, unknown>): string;
  registerAttempt(context: PhoneObservationContext, data?: Record<string, unknown>): string;
  snapshot(): StoredPhoneObservation[];
}
export interface RunnerController {
  store: RunnerStore; getOwner?(): string | null;
  withOwnership<T>(owner: 'sweep' | 'acceptance', work: () => Promise<T>): Promise<T>;
  runOperation<T>(options: {owner: 'sweep' | 'acceptance'; attemptID: string; stage: AttemptStage; signal?: AbortSignal}, work: (id: string) => Promise<T>): Promise<T>;
  configuration(): Promise<string>; flush(): Promise<void>; collect(): Promise<unknown>; closeManualConversation?(reason: string): void;
}
export interface RunnerNative {
  close?(): Promise<void>; open?(): Promise<unknown>; setResourceSamplingEnabled?(enabled: boolean): Promise<void>;
  listModels(): Promise<Model[]>; listReasoningModels(): Promise<Model[]>;
  configure(settings: Settings): Promise<unknown>; configureReasoning(id: string, threads: number): Promise<unknown>;
  transcribeFile(uri: string, attemptID?: string): Promise<{transcription: {result: {text: string; status: string}}}>;
  respond(text: string, requestID: string, attemptID?: string): Promise<unknown>;
  resetConversation(): Promise<unknown>; cancel(): Promise<void>; setBenchmarkActive(active: boolean): Promise<void>;
  inspect(): Promise<{busy: boolean; closed: boolean}>;
}
export interface RunnerDependencies {
  controller: RunnerController; native: RunnerNative;
  save(sweep: BenchmarkSweep): void; verify(dataset: BenchmarkDataset): Promise<void>;
  reserve(bytes: number): void; now(): number; monotonic(): number; delay(ms: number): Promise<void>;
  progress?(sweep: BenchmarkSweep): void;
  scheduleInterval?(callback: () => void, milliseconds: number): () => void;
}
export const DEFAULT_PROTOCOL: BenchmarkProtocol = {
  version: 1, automatic_retries: 0, mode: 'fixed_script', repetitions: 2, seed: 1, cooldown_ms: 1000, warmup: true,
  upload_policy: 'conversation_boundary', thermal_policy: 'fixed_interval',
  conditions: {brightness: 'unrecorded', power_mode: 'unrecorded', charging: 'unrecorded'},
};
function shuffle<T>(items: readonly T[], random: () => number): T[] {
  const result = [...items]; for (let i=result.length-1;i>0;i--) { const j=Math.floor(random()*(i+1)); [result[i],result[j]]=[result[j],result[i]]; } return result;
}
export function expandMatrix(configurations: number, cases: readonly string[], protocol: BenchmarkProtocol): Omit<BenchmarkJob,'id'>[] {
  if (!Number.isInteger(configurations) || configurations < 1 || configurations > 16 || !cases.length || cases.length > 10 || new Set(cases).size !== cases.length) throw new Error('Select 1–16 unique configurations and 1–10 cases.');
  if (!Number.isInteger(protocol.repetitions) || protocol.repetitions < 1 || protocol.repetitions > 20 || configurations * cases.length * protocol.repetitions > 240) throw new Error('A sweep is limited to 240 executions and 20 repetitions.');
  if (!Number.isSafeInteger(protocol.seed) || protocol.seed < 0 || protocol.seed > 0xffffffff || !Number.isInteger(protocol.cooldown_ms) || protocol.cooldown_ms < 0 || protocol.cooldown_ms > 60000) throw new Error('Use a 32-bit seed and a cooldown between 0 and 60000 ms.');
  let seed=protocol.seed >>> 0; const random=() => { seed=(seed+0x6d2b79f5)|0; let t=Math.imul(seed^(seed>>>15),1|seed); t^=t+Math.imul(t^(t>>>7),61|t); return ((t^(t>>>14))>>>0)/4294967296; };
  const jobs: Omit<BenchmarkJob,'id'>[]=[]; let block=0;
  for (let repetition=1;repetition<=protocol.repetitions;repetition++) for (const configuration_index of shuffle(Array.from({length:configurations},(_,i)=>i),random)) {
    for (const case_id of cases) jobs.push({configuration_index,case_id,repetition,block,status:'planned',executions:[]}); block++;
  }
  return jobs;
}
function stopped(): Error { return Object.assign(new Error('Benchmark cancelled.'),{code:'cancelled'}); }

export function protocolRevision(protocol: BenchmarkProtocol): string {
  const canonical = (value: unknown): string => {
    if (Array.isArray(value)) return `[${value.map(canonical).join(',')}]`;
    if (value && typeof value === 'object') return `{${Object.entries(value).filter(([, v]) => v !== undefined).sort(([a], [b]) => a.localeCompare(b)).map(([key, v]) => `${JSON.stringify(key)}:${canonical(v)}`).join(',')}}`;
    return JSON.stringify(value) ?? 'null';
  };
  return sha256(new TextEncoder().encode(canonical(protocol)));
}

/** One sequential scheduler. Native work only runs inside the controller's shared ownership lease. */
export function createBenchmarkRunner(deps: RunnerDependencies) {
  const {controller, native}=deps, store=controller.store;
  let active: Promise<void> | null=null, pauseRequested=false, cancellation=new AbortController();
  const persist=(sweep: BenchmarkSweep) => { try { deps.save(sweep); } catch(error) { throw Object.assign(error instanceof Error ? error : new Error('Benchmark storage failed.'), {code:'storage_failure'}); } deps.progress?.(sweep); };
  const append=(kind: ObservationKind,id: string,context: PhoneObservationContext,data: Record<string,unknown>) => store.append(kind,id,context,data);
  function assertRunning() { if (cancellation.signal.aborted) throw stopped(); }
  function context(sweep: BenchmarkSweep,job?: BenchmarkJob): PhoneObservationContext { return {sweep_id:sweep.id,...(job?{job_id:job.id,configuration_id:job.configuration_id}:{})}; }
  function endJob(sweep: BenchmarkSweep,job: BenchmarkJob,status: ExecutionStatus,reason?: string) {
    job.status=status; job.reason_code=reason;
    append('job',job.id,context(sweep,job),{status,finished_at_ms:deps.now(),...(reason?{reason_code:reason}:{})}); persist(sweep);
  }
  function plan(dataset: BenchmarkDataset, configurations: BenchmarkConfiguration[], protocol: BenchmarkProtocol, source: BenchmarkSweep['source']='sweep', acceptanceID?: string): BenchmarkSweep {
    const frozen=freezeDataset(dataset); if (frozen.revision !== dataset.revision) throw new Error('Dataset revision does not match its pinned manifest.');
    const combinations = new Set(configurations.map(c=>JSON.stringify([c.asr.sha256,c.reasoning?.sha256,c.settings,c.reasoning_threads])));
    if (combinations.size !== configurations.length) throw new Error('Remove duplicate model configurations.');
    if (configurations.some(c=>c.settings.modelID!==c.asr.id || !c.settings.metricsEnabled || c.settings.maxAudioSeconds < Math.max(...dataset.cases.flatMap(x=>x.turns.map(t=>t.duration_ms/1000))) || (protocol.mode==='fixed_script' && !c.reasoning))) throw new Error('Configurations need enabled collection, the selected ASR model, sufficient audio duration, and a reasoning model for fixed scripts.');
    const matrix=expandMatrix(configurations.length,dataset.cases.map(c=>c.id),protocol); deps.reserve(32*1024*1024+matrix.length*128*1024);
    store.assertCanStart?.();
    const protocol_revision=protocolRevision(protocol);
    const id=store.registerSweep({status:'planned',source,planned_jobs:matrix.length,...datasetMetadata(dataset),protocol,protocol_revision});
    const sweep: BenchmarkSweep={schema_version:1,id,source,created_at_ms:deps.now(),status:'planned',dataset:frozen,configurations:JSON.parse(JSON.stringify(configurations)) as BenchmarkConfiguration[],protocol:{...protocol,conditions:{...protocol.conditions}},protocol_revision,jobs:[],acceptance_id:acceptanceID};
    sweep.jobs=matrix.map(job=>({...job,id:store.registerJob({sweep_id:id},{status:'planned',source,case_id:job.case_id,repetition:job.repetition,configuration_index:job.configuration_index,block:job.block})}));
    // The complete matrix must be durable before any native call can be admitted.
    store.pin?.(id,true); persist(sweep); return sweep;
  }
  function recover(sweep: BenchmarkSweep) {
    if(controller.getOwner?.())return;
    if (sweep.status!=='running' && !sweep.jobs.some(j=>j.status==='running')) return;
    const latest=new Map<string,PhoneObservation>();
    for(const {record} of store.snapshot()) if(record.context.sweep_id===sweep.id) { const key=record.kind+record.entity_id; if((latest.get(key)?.revision??0)<record.revision) latest.set(key,record); }
    for(const job of sweep.jobs){const observed=latest.get('job'+job.id);if(observed&&['succeeded','failed','cancelled','skipped'].includes(String(observed.data.status)))job.status=observed.data.status as ExecutionStatus;}
    const observedSweep=latest.get('sweep'+sweep.id);
    if(observedSweep&&['succeeded','failed','cancelled'].includes(String(observedSweep.data.status))){sweep.status=observedSweep.data.status as BenchmarkSweep['status'];sweep.current_job_id=undefined;sweep.current_stage=undefined;persist(sweep);return;}
    for(const record of latest.values()) if(['conversation','turn','attempt'].includes(record.kind) && ['running','active','planned'].includes(String(record.data.status))) append(record.kind,record.entity_id,record.context,{status:'interrupted',finished_at_ms:deps.now(),reason_code:'runtime_lost'});
    for(const job of sweep.jobs) if(job.status==='running') { append('job',job.id,context(sweep,job),{status:'running',reason_code:'runtime_lost',next_execution_required:true}); job.status='planned'; job.reason_code='rerun_after_interruption'; }
    sweep.status='paused'; sweep.current_job_id=undefined; sweep.current_stage=undefined;
    append('sweep',sweep.id,{sweep_id:sweep.id},{status:'running',pause_reason:'runtime_lost'}); persist(sweep);
  }
  async function operation<T>(sweep: BenchmarkSweep,ctx: PhoneObservationContext,stage: AttemptStage,engine: string,work:(id:string)=>Promise<T>): Promise<T> {
    assertRunning(); sweep.current_stage=stage; persist(sweep);
    const attemptID=store.registerAttempt(ctx,{status:'planned',stage,engine,source:sweep.source});
    return controller.runOperation({owner:sweep.source,attemptID,stage,signal:cancellation.signal},work);
  }
  async function prepare(sweep: BenchmarkSweep,job: BenchmarkJob) {
    const config=sweep.configurations[job.configuration_index], ctx=context(sweep,job);
    if (native.close && native.open) {
      // Release both previous models before loading another pair. Keeping the
      // old reasoning model resident while replacing ASR can exhaust phone RAM.
      await controller.collect(); await native.close(); await native.open();
      await native.setResourceSamplingEnabled?.(sweep.protocol.sample_resources ?? true);
      await native.setBenchmarkActive(true); await controller.configuration();
    }
    await operation(sweep,ctx,'loading','asr',()=>native.configure(config.settings)); assertRunning();
    if(config.reasoning && sweep.protocol.mode==='fixed_script') await operation(sweep,ctx,'loading','reasoning',()=>native.configureReasoning(config.reasoning!.id,config.reasoning_threads));
    assertRunning(); const configurationID=await controller.configuration();
    for(const entry of sweep.jobs.filter(j=>j.block===job.block)) { entry.configuration_id=configurationID; append('job',entry.id,context(sweep,entry),{configuration_index:entry.configuration_index}); }
    persist(sweep);
    if(sweep.protocol.warmup) {
      await native.resetConversation();
      const audio=sweep.dataset.cases[0].turns[0];
      const result=await operation(sweep,context(sweep,job),'warmup','asr',id=>native.transcribeFile(audio.uri,id)); assertRunning();
      if(config.reasoning && sweep.protocol.mode==='fixed_script') await operation(sweep,context(sweep,job),'warmup','reasoning',id=>native.respond(result.transcription.result.text,store.newID(),id));
    }
    assertRunning(); await native.resetConversation();
  }
  function finalize(ctx: PhoneObservationContext) {
    const relevant=store.snapshot().map(x=>x.record).filter(r=>(!ctx.sweep_id||r.context.sweep_id===ctx.sweep_id)&&(!ctx.conversation_id||r.context.conversation_id===ctx.conversation_id));
    const attempts=new Set(relevant.filter(r=>r.kind==='attempt').map(r=>r.entity_id));
    const finals=new Map<string,PhoneObservation>(); for(const r of relevant) if(r.kind==='finalization'&&r.context.attempt_id && (finals.get(r.context.attempt_id)?.revision??0)<r.revision) finals.set(r.context.attempt_id,r);
    const batches=new Map(relevant.filter(r=>r.kind==='metric_batch').map(r=>[r.entity_id,r]));
    const lost=[...finals.values()].reduce((n,r)=>n+Number(r.data.dropped_events??0),0);
    const data={status:[...attempts].every(id=>finals.get(id)?.data.status==='complete')?'complete':'partial',event_count:[...batches.values()].reduce((n,r)=>n+((r.data.batch as {events?:unknown[]})?.events?.length??0),0),dropped_events:lost,observed_attempts:attempts.size};
    if(store.finalize) store.finalize(ctx,data); else append('finalization',ctx.conversation_id??ctx.sweep_id!,ctx,data);
  }
  function activeDuration(ctx: PhoneObservationContext) {
    const attempts=new Map<string,PhoneObservation>();
    for(const {record:r} of store.snapshot()) if(r.kind==='attempt' && r.context.conversation_id===ctx.conversation_id && (!ctx.turn_id||r.context.turn_id===ctx.turn_id) && ['transcription','reasoning'].includes(String(r.data.stage)) && (attempts.get(r.entity_id)?.revision??0)<r.revision) attempts.set(r.entity_id,r);
    const rows=[...attempts.values()];
    return rows.length && rows.every(r=>typeof r.data.duration_ms==='number' && Number.isFinite(r.data.duration_ms)) ? rows.reduce((n,r)=>n+Number(r.data.duration_ms),0) : null;
  }
  async function executeConversation(sweep: BenchmarkSweep,job: BenchmarkJob) {
    assertRunning(); const entry=sweep.dataset.cases.find(c=>c.id===job.case_id)!;
    await native.resetConversation(); assertRunning();
    job.status='running'; sweep.current_job_id=job.id;
    const started=deps.monotonic(), startedAt=deps.now();
    const conversationID=store.registerConversation(context(sweep,job),{status:'running',started_at_ms:startedAt,source:sweep.source,case_id:job.case_id,repetition:job.repetition,expected_turns:entry.turns.length,dataset_revision:sweep.dataset.revision});
    job.executions.push(conversationID); const ctx={...context(sweep,job),conversation_id:conversationID};
    append('job',job.id,context(sweep,job),{status:'running',started_at_ms:startedAt,execution_count:job.executions.length,latest_execution_id:conversationID}); persist(sweep);
    const turnIDs=entry.turns.map((_,index)=>store.registerTurn(ctx,{status:'planned',turn_index:index,source:sweep.source,input_kind:'fixed_wav'}));
    let nextTurn=0, finalStatus: ExecutionStatus='succeeded', reason: string|undefined;
    try {
      for(let index=0;index<entry.turns.length;index++) {
        assertRunning(); nextTurn=index;
        const turnCtx={...ctx,turn_id:turnIDs[index]}, start=deps.monotonic();
        append('turn',turnIDs[index],turnCtx,{status:'running',started_at_ms:deps.now()});
        const result=await operation(sweep,turnCtx,'transcription','asr',id=>native.transcribeFile(entry.turns[index].uri,id)); assertRunning();
        // Rust's serialized enum is "Speech"; native metric labels use "speech".
        if(result.transcription.result.status.toLowerCase()!=='speech') throw Object.assign(new Error('Transcription did not produce usable speech.'),{code:'transcription_rejected'});
        if(!result.transcription.result.text.trim()) throw Object.assign(new Error('No speech was transcribed.'),{code:'empty_transcript'});
        if(sweep.protocol.mode==='fixed_script') await operation(sweep,turnCtx,'reasoning','reasoning',id=>native.respond(result.transcription.result.text,store.newID(),id));
        assertRunning(); append('turn',turnIDs[index],turnCtx,{status:'succeeded',finished_at_ms:deps.now(),duration_ms:activeDuration(turnCtx),wall_duration_ms:Math.max(0,deps.monotonic()-start),timer_boundary:'sum_native_call_including_bridge',wall_timer_boundary:'turn_scheduling_and_collection'}); nextTurn=index+1;
      }
    } catch(error) {
      const code=error && typeof error==='object' && 'code' in error?String(error.code):'';
      finalStatus=cancellation.signal.aborted?'cancelled':code==='interrupted'?'interrupted':'failed'; reason=cancellation.signal.aborted?'user_cancelled':code==='interrupted'?'app_background':'stage_failed';
      if(finalStatus==='interrupted')pauseRequested=true;
      if(nextTurn<turnIDs.length) append('turn',turnIDs[nextTurn],{...ctx,turn_id:turnIDs[nextTurn]},{status:finalStatus,finished_at_ms:deps.now(),duration_ms:activeDuration({...ctx,turn_id:turnIDs[nextTurn]}),timer_boundary:'sum_native_call_including_bridge',reason_code:reason});
      for(let index=nextTurn+1;index<turnIDs.length;index++) append('turn',turnIDs[index],{...ctx,turn_id:turnIDs[index]},{status:'skipped',finished_at_ms:deps.now(),reason_code:'prior_turn_failed'});
      if(error && typeof error==='object' && 'code' in error && ['archive_full','storage_failure','collection_unavailable'].includes(String(error.code))) throw error;
    }
    append('conversation',conversationID,ctx,{status:finalStatus,finished_at_ms:deps.now(),duration_ms:activeDuration(ctx),wall_duration_ms:Math.max(0,deps.monotonic()-started),timer_boundary:'sum_native_call_including_bridge',wall_timer_boundary:'conversation_wall',...(reason?{reason_code:reason}:{})});
    await controller.collect(); finalize(ctx);
    if(finalStatus==='interrupted'){job.status='planned';append('job',job.id,context(sweep,job),{status:'running',pause_reason:reason,next_execution_required:true});persist(sweep);}else endJob(sweep,job,finalStatus,reason);
    if(sweep.protocol.upload_policy==='conversation_boundary') await controller.flush();
  }
  async function execute(sweep: BenchmarkSweep) {
    await controller.withOwnership(sweep.source,async()=>{
      let stopUploads: (() => void) | undefined;
      let pendingUpload: Promise<void> | null = null;
      const upload = () => {
        if (pendingUpload) return;
        pendingUpload = controller.flush().catch(() => {}).finally(() => { pendingUpload = null; });
      };
      try {
        await controller.configuration();
        await deps.verify(sweep.dataset); deps.reserve(32*1024*1024+sweep.jobs.filter(j=>j.status==='planned').length*128*1024);
        const asr = await native.listModels();
        const reasoning = await native.listReasoningModels();
        for(const config of sweep.configurations) if(!asr.some(m=>m.id===config.asr.id&&m.sha256===config.asr.sha256)|| (config.reasoning&&!reasoning.some(m=>m.id===config.reasoning?.id&&m.sha256===config.reasoning.sha256))) throw new Error('A selected model artifact is unavailable; no substitute is allowed.');
        await native.setBenchmarkActive(true); await native.resetConversation(); controller.closeManualConversation?.('benchmark_started'); sweep.status='running'; append('sweep',sweep.id,{sweep_id:sweep.id},{status:'running',started_at_ms:deps.now()}); persist(sweep);
        if (sweep.protocol.upload_policy === 'live') {
          // The root uploader defers while this lease owns inference. Live mode
          // explicitly allows bounded uploads alongside native work instead.
          const schedule = deps.scheduleInterval ?? ((callback, milliseconds) => {
            const timer = setInterval(callback, milliseconds);
            return () => clearInterval(timer);
          });
          stopUploads = schedule(upload, 5000);
          upload();
        }
        let loadedBlock=-1;
        for(const job of sweep.jobs) {
          if(job.status!=='planned') continue;
          if(pauseRequested||cancellation.signal.aborted) break;
          sweep.current_job_id=job.id; persist(sweep);
          if(job.block!==loadedBlock) {
            if(loadedBlock!==-1 && sweep.protocol.cooldown_ms) { sweep.current_stage='cooldown'; persist(sweep); await deps.delay(sweep.protocol.cooldown_ms); }
            if(pauseRequested||cancellation.signal.aborted) break;
            try { await prepare(sweep,job); loadedBlock=job.block; }
            catch(error) {
              if(cancellation.signal.aborted) break;
              // Missing telemetry storage must pause, never consume more cases.
              if(error && typeof error==='object' && 'code' in error && ['archive_full','storage_failure','collection_unavailable'].includes(String(error.code))) throw error;
              for(const blocked of sweep.jobs.filter(j=>j.block===job.block&&j.status==='planned')) endJob(sweep,blocked,'failed','configuration_failed');
              continue;
            }
          }
          await executeConversation(sweep,job);
          if(cancellation.signal.aborted) break;
          const health=await native.inspect(); if(health.busy||health.closed) throw new Error('Native engine has not returned to an idle usable state.');
        }
        if(cancellation.signal.aborted) { for(const job of sweep.jobs) if(job.status==='planned') endJob(sweep,job,'skipped','sweep_cancelled'); sweep.status='cancelled'; }
        else if(sweep.jobs.some(j=>j.status==='planned')) sweep.status='paused';
        else sweep.status=sweep.jobs.some(j=>j.status==='failed')?'failed':'succeeded';
        append('sweep',sweep.id,{sweep_id:sweep.id},{status:sweep.status==='paused'?'running':sweep.status,...(sweep.status==='paused'?{pause_requested:true}:{finished_at_ms:deps.now()})});
        if(sweep.status!=='paused') finalize({sweep_id:sweep.id});
      } catch(error) { sweep.status='paused'; append('sweep',sweep.id,{sweep_id:sweep.id},{status:'running',pause_reason:'preflight_or_storage_failure'}); throw error; }
      finally {
        stopUploads?.();
        await pendingUpload;
        if (sweep.protocol.upload_policy === 'live') await controller.flush().catch(() => {});
        // The original operation promises have settled before ownership/display state is released.
        sweep.current_stage=undefined; sweep.current_job_id=undefined;
        try { await native.resetConversation(); } finally { await native.setBenchmarkActive(false); persist(sweep); }
      }
    });
  }
  return {
    plan,recover,
    start(sweep: BenchmarkSweep): Promise<void> {
      if(active||controller.getOwner?.()) return Promise.reject(new Error('A native operation is already running.'));
      if(['succeeded','failed','cancelled'].includes(sweep.status)) return Promise.reject(new Error('This sweep is finished. Create a new sweep to rerun it.'));
      recover(sweep); if(['succeeded','failed','cancelled'].includes(sweep.status))return Promise.resolve(); pauseRequested=false; cancellation=new AbortController();
      active=execute(sweep).finally(()=>{active=null;}); return active;
    },
    pause() { pauseRequested=true; },
    async cancel() { if(!active) return; cancellation.abort(); const settlement=active; try { await native.cancel(); } finally { await settlement; } },
    get running() { return active!==null; },
  };
}
