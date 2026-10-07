import Darwin
import IOKit
import IOKit.ps
import SwiftUI

struct BatteryInfo {
    var percent: Int
    var charging: Bool
    var onAC: Bool
    var cycles: Int?
    /// Current full-charge capacity vs. design capacity.
    var health: Double?
}

struct ProcessUsage: Identifiable {
    let id = UUID()
    let name: String
    let cpu: Double
    let memory: UInt64
}

@MainActor
final class StatusModel: ObservableObject {
    @Published var cpu: Double = 0
    @Published var cores: [Double] = []
    @Published var memUsed: UInt64 = 0
    @Published var memTotal: UInt64 = ProcessInfo.processInfo.physicalMemory
    @Published var swapUsed: UInt64 = 0
    @Published var disk = DiskSpace.current()
    @Published var battery: BatteryInfo?
    @Published var top: [ProcessUsage] = []
    @Published var uptime: TimeInterval = ProcessInfo.processInfo.systemUptime

    private var previousTicks: [[UInt32]] = []
    private var timer: Timer?

    func start() {
        tick()
        timer = Timer.scheduledTimer(withTimeInterval: 2, repeats: true) { [weak self] _ in
            Task { @MainActor in self?.tick() }
        }
    }

    func stop() {
        timer?.invalidate()
        timer = nil
    }

    private func tick() {
        sampleCPU()
        sampleMemory()
        disk = DiskSpace.current()
        battery = Self.readBattery()
        uptime = ProcessInfo.processInfo.systemUptime
        Task.detached {
            let procs = Self.topProcesses()
            await MainActor.run { self.top = procs }
        }
    }

    private func sampleCPU() {
        var count: natural_t = 0
        var info: processor_info_array_t?
        var infoCount: mach_msg_type_number_t = 0
        guard host_processor_info(mach_host_self(), PROCESSOR_CPU_LOAD_INFO, &count, &info, &infoCount) == KERN_SUCCESS,
              let info else { return }
        defer {
            vm_deallocate(mach_task_self_, vm_address_t(bitPattern: info),
                          vm_size_t(Int(infoCount) * MemoryLayout<integer_t>.stride))
        }
        var ticks: [[UInt32]] = []
        for i in 0..<Int(count) {
            let base = Int(CPU_STATE_MAX) * i
            ticks.append((0..<Int(CPU_STATE_MAX)).map { UInt32(bitPattern: info[base + $0]) })
        }
        if previousTicks.count == ticks.count {
            var usages: [Double] = []
            for (now, before) in zip(ticks, previousTicks) {
                let d = zip(now, before).map { Double($0 &- $1) }
                let idle = d[Int(CPU_STATE_IDLE)]
                let total = d.reduce(0, +)
                usages.append(total > 0 ? 1 - idle / total : 0)
            }
            cores = usages
            cpu = usages.isEmpty ? 0 : usages.reduce(0, +) / Double(usages.count)
        }
        previousTicks = ticks
    }

    private func sampleMemory() {
        var stats = vm_statistics64()
        var count = mach_msg_type_number_t(MemoryLayout<vm_statistics64_data_t>.size / MemoryLayout<integer_t>.size)
        let ok = withUnsafeMutablePointer(to: &stats) {
            $0.withMemoryRebound(to: integer_t.self, capacity: Int(count)) {
                host_statistics64(mach_host_self(), HOST_VM_INFO64, $0, &count)
            }
        }
        guard ok == KERN_SUCCESS else { return }
        let page = UInt64(vm_kernel_page_size)
        // Matches Activity Monitor's "Memory Used": app + wired + compressed.
        let app = UInt64(stats.internal_page_count) - UInt64(stats.purgeable_count)
        memUsed = (app + UInt64(stats.wire_count) + UInt64(stats.compressor_page_count)) * page

        var swap = xsw_usage()
        var size = MemoryLayout<xsw_usage>.size
        if sysctlbyname("vm.swapusage", &swap, &size, nil, 0) == 0 { swapUsed = swap.xsu_used }
    }

    nonisolated static func readBattery() -> BatteryInfo? {
        guard let blob = IOPSCopyPowerSourcesInfo()?.takeRetainedValue(),
              let list = IOPSCopyPowerSourcesList(blob)?.takeRetainedValue() as? [CFTypeRef] else { return nil }
        for source in list {
            guard let d = IOPSGetPowerSourceDescription(blob, source)?.takeUnretainedValue() as? [String: Any],
                  (d[kIOPSTypeKey] as? String) == kIOPSInternalBatteryType,
                  let cur = d[kIOPSCurrentCapacityKey] as? Int,
                  let max = d[kIOPSMaxCapacityKey] as? Int, max > 0 else { continue }
            var info = BatteryInfo(percent: cur * 100 / max,
                                   charging: d[kIOPSIsChargingKey] as? Bool ?? false,
                                   onAC: (d[kIOPSPowerSourceStateKey] as? String) == kIOPSACPowerValue)
            let svc = IOServiceGetMatchingService(kIOMainPortDefault, IOServiceMatching("AppleSmartBattery"))
            if svc != 0 {
                func prop(_ k: String) -> Int? {
                    IORegistryEntryCreateCFProperty(svc, k as CFString, kCFAllocatorDefault, 0)?.takeRetainedValue() as? Int
                }
                info.cycles = prop("CycleCount")
                if let full = prop("AppleRawMaxCapacity") ?? prop("NominalChargeCapacity"),
                   let design = prop("DesignCapacity"), design > 0 {
                    info.health = min(1, Double(full) / Double(design))
                }
                IOObjectRelease(svc)
            }
            return info
        }
        return nil
    }

    nonisolated static func topProcesses() -> [ProcessUsage] {
        let p = Process()
        p.executableURL = URL(fileURLWithPath: "/bin/ps")
        p.arguments = ["-Aceo", "pcpu=,rss=,comm=", "-r"]
        let out = Pipe()
        p.standardOutput = out
        p.standardError = FileHandle.nullDevice
        guard (try? p.run()) != nil else { return [] }
        let data = out.fileHandleForReading.readDataToEndOfFile()
        p.waitUntilExit()
        return String(decoding: data, as: UTF8.self)
            .split(separator: "\n")
            .prefix(6)
            .compactMap { line in
                let f = line.split(separator: " ", maxSplits: 2, omittingEmptySubsequences: true)
                guard f.count == 3, let cpu = Double(f[0]), let rss = UInt64(f[1]) else { return nil }
                return ProcessUsage(name: String(f[2]), cpu: cpu, memory: rss * 1024)
            }
    }
}

struct StatusView: View {
    @StateObject private var model = StatusModel()

    private var uptimeText: String {
        let f = DateComponentsFormatter()
        f.allowedUnits = [.day, .hour, .minute]
        f.unitsStyle = .abbreviated
        return f.string(from: model.uptime) ?? ""
    }

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 18) {
                VStack(alignment: .leading, spacing: 4) {
                    Text("Status").font(.system(size: 30, weight: .bold, design: .rounded))
                    Text("Up for \(uptimeText)").font(.system(size: 13)).foregroundStyle(.secondary)
                }
                LazyVGrid(columns: [GridItem(.flexible(), spacing: 14), GridItem(.flexible(), spacing: 14)], spacing: 14) {
                    StatCard(title: "CPU", symbol: "cpu", tint: .blue,
                             value: String(format: "%.0f%%", model.cpu * 100), fraction: model.cpu) {
                        HStack(alignment: .bottom, spacing: 2) {
                            ForEach(Array(model.cores.enumerated()), id: \.offset) { _, c in
                                RoundedRectangle(cornerRadius: 2)
                                    .fill(Color.blue.opacity(0.25 + 0.75 * c))
                                    .frame(height: max(3, 30 * c))
                            }
                        }
                        .frame(height: 30, alignment: .bottom)
                    }
                    StatCard(title: "Memory", symbol: "memorychip", tint: .purple,
                             value: formatBytes(model.memUsed),
                             fraction: Double(model.memUsed) / Double(max(model.memTotal, 1))) {
                        Text("of \(formatBytes(model.memTotal))" + (model.swapUsed > 0 ? " · \(formatBytes(model.swapUsed)) swap" : ""))
                            .font(.system(size: 12)).foregroundStyle(.secondary)
                    }
                    StatCard(title: "Disk", symbol: "internaldrive", tint: .teal,
                             value: "\(formatBytes(model.disk.free)) free",
                             fraction: Double(model.disk.used) / Double(max(model.disk.total, 1))) {
                        Text("\(formatBytes(model.disk.used)) used of \(formatBytes(model.disk.total))")
                            .font(.system(size: 12)).foregroundStyle(.secondary)
                    }
                    if let b = model.battery {
                        StatCard(title: "Battery", symbol: b.charging ? "battery.100.bolt" : "battery.75", tint: .green,
                                 value: "\(b.percent)%", fraction: Double(b.percent) / 100) {
                            Text([b.charging ? "Charging" : (b.onAC ? "On power" : "On battery"),
                                  b.health.map { String(format: "%.0f%% health", $0 * 100) },
                                  b.cycles.map { "\($0) cycles" }].compactMap { $0 }.joined(separator: " · "))
                                .font(.system(size: 12)).foregroundStyle(.secondary)
                        }
                    }
                }

                VStack(alignment: .leading, spacing: 8) {
                    Text("Busiest apps").font(.system(size: 14, weight: .semibold))
                    VStack(spacing: 0) {
                        ForEach(model.top) { p in
                            HStack {
                                Text(p.name).font(.system(size: 13)).lineLimit(1)
                                Spacer()
                                Text(String(format: "%.1f%% CPU", p.cpu)).foregroundStyle(.secondary)
                                    .frame(width: 90, alignment: .trailing)
                                Text(formatBytes(p.memory)).foregroundStyle(.secondary)
                                    .frame(width: 80, alignment: .trailing)
                            }
                            .font(.system(size: 12, design: .rounded)).monospacedDigit()
                            .padding(.horizontal, 14).padding(.vertical, 8)
                            if p.id != model.top.last?.id { Divider().padding(.leading, 14) }
                        }
                    }
                    .background(RoundedRectangle(cornerRadius: 14, style: .continuous).fill(Color(nsColor: .controlBackgroundColor)))
                    .overlay(RoundedRectangle(cornerRadius: 14, style: .continuous).strokeBorder(Color.primary.opacity(0.06)))
                }
            }
            .padding(.horizontal, 28).padding(.top, 22).padding(.bottom, 24)
        }
        .onAppear { model.start() }
        .onDisappear { model.stop() }
    }
}

private struct StatCard<Detail: View>: View {
    let title: String
    let symbol: String
    let tint: Color
    let value: String
    let fraction: Double
    @ViewBuilder let detail: Detail

    var body: some View {
        HStack(spacing: 16) {
            ZStack {
                Circle().stroke(tint.opacity(0.15), lineWidth: 7)
                Circle().trim(from: 0, to: min(1, max(0.01, fraction)))
                    .stroke(tint, style: StrokeStyle(lineWidth: 7, lineCap: .round))
                    .rotationEffect(.degrees(-90))
                    .animation(.smooth, value: fraction)
                Image(systemName: symbol).font(.system(size: 16, weight: .semibold)).foregroundStyle(tint)
            }
            .frame(width: 58, height: 58)
            VStack(alignment: .leading, spacing: 4) {
                Text(title).font(.system(size: 12, weight: .medium)).foregroundStyle(.secondary)
                Text(value).font(.system(size: 20, weight: .bold, design: .rounded)).monospacedDigit()
                    .contentTransition(.numericText())
                detail
            }
            Spacer(minLength: 0)
        }
        .padding(16)
        .frame(maxWidth: .infinity, minHeight: 110, alignment: .leading)
        .background(RoundedRectangle(cornerRadius: 14, style: .continuous).fill(Color(nsColor: .controlBackgroundColor)))
        .overlay(RoundedRectangle(cornerRadius: 14, style: .continuous).strokeBorder(Color.primary.opacity(0.06)))
    }
}
