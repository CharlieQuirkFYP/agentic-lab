import Foundation
import Darwin
#if canImport(UIKit)
import UIKit
#endif

struct NativeAttempt: Codable, Sendable {
    let attempt_id: String
    let stage: String
    // The additive ABIs use the caller identity as the native run identity.
    let native_run_id: String
}

struct NativeMetricValue: Encodable, Sendable {
    let name: String
    let unit: String
    let scope: String
    let source: String
    var number: Double? = nil
    var text: String? = nil
    var unavailable_reason: String? = nil
    enum CodingKeys: String, CodingKey { case name, unit, scope, source, value, unavailable_reason }
    func encode(to encoder: Encoder) throws {
        var out = encoder.container(keyedBy: CodingKeys.self)
        try out.encode(name, forKey: .name); try out.encode(unit, forKey: .unit)
        try out.encode(scope, forKey: .scope); try out.encode(source, forKey: .source)
        if let number { try out.encode(number, forKey: .value) }
        else if let text { try out.encode(text, forKey: .value) }
        else { try out.encodeNil(forKey: .value) }
        try out.encodeIfPresent(unavailable_reason, forKey: .unavailable_reason)
    }
}

struct NativeResourceSample: Encodable, Sendable {
    let runtime_session_id: String
    let attempt_id: String?
    let native_run_id: String?
    let stage: String
    let timestamp_ms: Double
    let monotonic_ms: Double
    let interval_ms: Double?
    let boundary: String
    let values: [NativeMetricValue]
    enum CodingKeys: String, CodingKey { case runtime_session_id, attempt_id, native_run_id, stage, timestamp_ms, monotonic_ms, interval_ms, boundary, values }
    func encode(to encoder: Encoder) throws {
        var out = encoder.container(keyedBy: CodingKeys.self)
        try out.encode(runtime_session_id, forKey: .runtime_session_id)
        try out.encodeIfPresent(attempt_id, forKey: .attempt_id); try out.encodeIfPresent(native_run_id, forKey: .native_run_id)
        try out.encode(stage, forKey: .stage); try out.encode(timestamp_ms, forKey: .timestamp_ms)
        try out.encode(monotonic_ms, forKey: .monotonic_ms)
        if let interval_ms { try out.encode(interval_ms, forKey: .interval_ms) } else { try out.encodeNil(forKey: .interval_ms) }
        try out.encode(boundary, forKey: .boundary); try out.encode(values, forKey: .values)
    }
}

struct NativeResourceObserver: Encodable, Sendable {
    let capacity_samples: Int
    let pending_samples: Int
    let dropped_samples: UInt64
    let interval_ms: Int
    let enabled: Bool
}

struct ResourceReading: Sendable {
    var cpuSeconds: Double?
    var residentBytes: Double?
    var footprintBytes: Double?
    var batteryLevel: Double?
    var batteryState: String?
    var thermalState: String
    var screenBrightness: Double? = nil
    var lowPowerMode: Bool = false
}

/// One serial, bounded sampler per runtime. Its queue is independent of JS and
/// inference. Samples describe the whole app process, never an isolated model.
final class NativeResourceSampler: @unchecked Sendable {
    static let intervalMS = 500
    private let queue = DispatchQueue(label: "org.agenticlab.pheme.resources", qos: .utility)
    private let sessionID: String
    private let capacity: Int
    private let read: @Sendable () -> ResourceReading
    private let clock: @Sendable () -> Double
    private let automatic: Bool
    private var timer: DispatchSourceTimer?
    private var samples: [NativeResourceSample] = []
    private var dropped: UInt64 = 0
    private var active: NativeAttempt?
    private var stage = "idle"
    private var previous: (time: Double, cpu: Double)?
    private var stopped = false
    private var enabled = true

    init(sessionID: String, capacity: Int = 512, automatic: Bool = true,
         read: @escaping @Sendable () -> ResourceReading = { NativeResourceProvider.read() },
         clock: @escaping @Sendable () -> Double = { ProcessInfo.processInfo.systemUptime * 1000 }) {
        self.sessionID = sessionID; self.capacity = max(1, capacity); self.read = read; self.clock = clock
        self.automatic = automatic
        if automatic { startTimer() }
    }

    private func startTimer() {
        guard automatic, timer == nil, enabled, !stopped else { return }
            let timer = DispatchSource.makeTimerSource(queue: queue)
            timer.schedule(deadline: .now() + .milliseconds(Self.intervalMS), repeating: .milliseconds(Self.intervalMS), leeway: .milliseconds(25))
            timer.setEventHandler { [weak self] in self?.sample(boundary: "periodic") }
            self.timer = timer; timer.resume()
    }

    deinit { timer?.cancel() }
    func begin(attempt: NativeAttempt?, stage: String) {
        queue.sync { self.active = attempt; self.stage = stage; previous = nil; sample(boundary: "start") }
    }
    func finish() {
        queue.sync { sample(boundary: "finish"); active = nil; stage = "idle"; previous = nil }
    }
    func tick() { queue.sync { sample(boundary: "periodic") } }
    func stop() { queue.sync { timer?.cancel(); timer = nil; stopped = true } }
    func setEnabled(_ enabled: Bool) {
        queue.sync {
            self.enabled = enabled
            previous = nil
            if enabled { startTimer() } else { timer?.cancel(); timer = nil }
        }
    }
    func status() -> NativeResourceObserver {
        queue.sync { observer() }
    }
    func drain() -> (NativeResourceObserver, [NativeResourceSample]) {
        queue.sync { let state = observer(); let retained = samples; samples.removeAll(keepingCapacity: true); return (state, retained) }
    }
    private func observer() -> NativeResourceObserver {
        NativeResourceObserver(capacity_samples: capacity, pending_samples: samples.count, dropped_samples: dropped, interval_ms: Self.intervalMS, enabled: enabled && !stopped)
    }
    private func sample(boundary: String) {
        guard !stopped, enabled else { return }
        let now = clock(); let reading = read()
        var interval: Double?
        var cpu: Double?
        if let old = previous, let cumulative = reading.cpuSeconds,
           now > old.time, now - old.time >= 100, cumulative >= old.cpu {
            interval = now - old.time
            cpu = (cumulative - old.cpu) / (now - old.time) * 100_000
        }
        previous = reading.cpuSeconds.map { (time: now, cpu: $0) }
        func numeric(_ name: String, _ unit: String, _ scope: String, _ source: String, _ value: Double?, _ reason: String) -> NativeMetricValue {
            NativeMetricValue(name: name, unit: unit, scope: scope, source: source, number: value, unavailable_reason: value == nil ? reason : nil)
        }
        let values = [
            numeric("cpu_percent", "percent_one_core", "process", "darwin.task_info", cpu, reading.cpuSeconds == nil ? "task_cpu_unavailable" : "insufficient_cpu_interval"),
            numeric("resident_memory_bytes", "bytes", "process", "darwin.mach_task_basic_info", reading.residentBytes, "task_memory_unavailable"),
            numeric("physical_footprint_bytes", "bytes", "process", "darwin.task_vm_info", reading.footprintBytes, "physical_footprint_unavailable"),
            numeric("battery_level", "percent", "device", "ios.UIDevice", reading.batteryLevel.map { $0 * 100 }, "battery_level_unavailable"),
            NativeMetricValue(name: "battery_state", unit: "state", scope: "device", source: "ios.UIDevice", text: reading.batteryState, unavailable_reason: reading.batteryState == nil ? "battery_state_unavailable" : nil),
            NativeMetricValue(name: "thermal_state", unit: "state", scope: "device", source: "ios.ProcessInfo", text: reading.thermalState),
            numeric("screen_brightness", "fraction", "device", "ios.UIScreen", reading.screenBrightness, "screen_brightness_unavailable"),
            NativeMetricValue(name: "low_power_mode", unit: "state", scope: "device", source: "ios.ProcessInfo", text: reading.lowPowerMode ? "enabled" : "disabled"),
            numeric("gpu_utilization", "percent", "device", "unavailable", nil, "no_validated_gpu_provider"),
            numeric("power", "watts", "device", "unavailable", nil, "no_validated_power_provider"),
            numeric("energy", "joules", "device", "unavailable", nil, "no_validated_energy_provider"),
        ]
        if samples.count == capacity { samples.removeFirst(); dropped += 1 }
        samples.append(NativeResourceSample(runtime_session_id: sessionID, attempt_id: active?.attempt_id,
            native_run_id: active?.native_run_id, stage: stage, timestamp_ms: (Date().timeIntervalSince1970 * 1000).rounded(.down),
            monotonic_ms: now, interval_ms: interval, boundary: boundary, values: values))
    }
}

enum NativeResourceProvider {
    static func read() -> ResourceReading {
        var basic = mach_task_basic_info(); var basicCount = mach_msg_type_number_t(MemoryLayout<mach_task_basic_info>.size / MemoryLayout<integer_t>.size)
        let basicCode = withUnsafeMutablePointer(to: &basic) { ptr in
            ptr.withMemoryRebound(to: integer_t.self, capacity: Int(basicCount)) { task_info(mach_task_self_, task_flavor_t(MACH_TASK_BASIC_INFO), $0, &basicCount) }
        }
        var threads = task_thread_times_info(); var threadCount = mach_msg_type_number_t(MemoryLayout<task_thread_times_info>.size / MemoryLayout<integer_t>.size)
        let threadCode = withUnsafeMutablePointer(to: &threads) { ptr in
            ptr.withMemoryRebound(to: integer_t.self, capacity: Int(threadCount)) { task_info(mach_task_self_, task_flavor_t(TASK_THREAD_TIMES_INFO), $0, &threadCount) }
        }
        var vm = task_vm_info_data_t(); var vmCount = mach_msg_type_number_t(MemoryLayout<task_vm_info_data_t>.size / MemoryLayout<integer_t>.size)
        let vmCode = withUnsafeMutablePointer(to: &vm) { ptr in
            ptr.withMemoryRebound(to: integer_t.self, capacity: Int(vmCount)) { task_info(mach_task_self_, task_flavor_t(TASK_VM_INFO), $0, &vmCount) }
        }
        func seconds(_ value: time_value_t) -> Double { Double(value.seconds) + Double(value.microseconds) / 1_000_000 }
        // basic times cover terminated threads; thread times cover live threads.
        let cpu = basicCode == KERN_SUCCESS && threadCode == KERN_SUCCESS ? seconds(basic.user_time) + seconds(basic.system_time) + seconds(threads.user_time) + seconds(threads.system_time) : nil
        let thermal: String
        switch ProcessInfo.processInfo.thermalState {
        case .nominal: thermal = "nominal"
        case .fair: thermal = "fair"
        case .serious: thermal = "serious"
        case .critical: thermal = "critical"
        @unknown default: thermal = "unknown"
        }
        #if canImport(UIKit)
        // UIKit observations are refreshed on the main queue. A blocked UI gives
        // unavailable battery values, never stale values assigned a new timestamp.
        let battery = NativeDeviceConditions.shared.readBattery()
        #else
        let battery: (Double?, String?, Double?) = (nil, nil, nil)
        #endif
        return ResourceReading(cpuSeconds: cpu, residentBytes: basicCode == KERN_SUCCESS ? Double(basic.resident_size) : nil,
            footprintBytes: vmCode == KERN_SUCCESS ? Double(vm.phys_footprint) : nil,
            batteryLevel: battery.0, batteryState: battery.1, thermalState: thermal,
            screenBrightness: battery.2, lowPowerMode: ProcessInfo.processInfo.isLowPowerModeEnabled)
    }
}

#if canImport(UIKit)
final class NativeDeviceConditions: @unchecked Sendable {
    static let shared = NativeDeviceConditions()
    private let lock = NSLock()
    private var battery: (level: Double?, state: String?, brightness: Double?, time: Double) = (nil, nil, nil, 0)
    private var refreshPending = false
    func readBattery() -> (Double?, String?, Double?) {
        // At most one outstanding main-queue refresh, even if the UI is blocked.
        let refresh = lock.withLock { if refreshPending { return false }; refreshPending = true; return true }
        if refresh { DispatchQueue.main.async { [self] in
            let device = UIDevice.current; device.isBatteryMonitoringEnabled = true
            let states: [UIDevice.BatteryState: String] = [.charging: "charging", .full: "full", .unplugged: "unplugged"]
            let level = device.batteryLevel >= 0 ? Double(device.batteryLevel) : nil
            let brightness = Double(UIScreen.main.brightness)
            lock.withLock { battery = (level, states[device.batteryState], brightness, ProcessInfo.processInfo.systemUptime); refreshPending = false }
        } }
        return lock.withLock {
            guard ProcessInfo.processInfo.systemUptime - battery.time < 1 else { return (nil, nil, nil) }
            return (battery.level, battery.state, battery.brightness)
        }
    }
}
#endif

/// Own only the benchmark's idle-timer override; closing/backgrounding releases it.
final class BenchmarkDisplayLease: @unchecked Sendable {
    private let lock = NSLock()
    private var enabled = false
    #if canImport(UIKit)
    private var previous: Bool?
    #endif
    var isActive: Bool { lock.withLock { enabled } }
    func set(_ active: Bool) {
        lock.withLock { enabled = active }
        #if canImport(UIKit)
        DispatchQueue.main.async { [self] in
            if lock.withLock({ enabled }) {
                if previous == nil { previous = UIApplication.shared.isIdleTimerDisabled }
                UIApplication.shared.isIdleTimerDisabled = true
            } else if let previous {
                UIApplication.shared.isIdleTimerDisabled = previous; self.previous = nil
            }
        }
        #endif
    }
}
