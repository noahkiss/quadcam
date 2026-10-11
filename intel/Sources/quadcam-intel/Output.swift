import Foundation

/// Every answer is one JSON object per line on stdout. Exit codes: 0 done, 1 failed,
/// 2 usage, 3 the feature is unavailable here (old macOS, locale or assets missing).
enum Exit: Int32 {
    case ok = 0
    case failed = 1
    case usage = 2
    case unavailable = 3
}

let encoder: JSONEncoder = {
    let e = JSONEncoder()
    e.outputFormatting = [.sortedKeys, .withoutEscapingSlashes]
    return e
}()

func emit<T: Encodable>(_ value: T) {
    guard let data = try? encoder.encode(value), var line = String(data: data, encoding: .utf8)
    else { return }
    line.append("\n")
    FileHandle.standardOutput.write(line.data(using: .utf8)!)
}

struct Problem: Encodable {
    let kind = "error"
    let code: String
    let message: String
}

func fail(_ code: String, _ message: String, _ exit: Exit) -> Never {
    emit(Problem(code: code, message: message))
    Foundation.exit(exit.rawValue)
}

/// The running macOS as `27.0.1`.
func osVersion() -> String {
    let v = ProcessInfo.processInfo.operatingSystemVersion
    return "\(v.majorVersion).\(v.minorVersion).\(v.patchVersion)"
}
