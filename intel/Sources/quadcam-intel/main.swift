import Foundation

/// The version of this helper's JSON. QuadCam reads it from `caps`.
let protocolVersion = 1

let usage = """
    quadcam-intel caps [--locale en-US]
    quadcam-intel asr FILE... [--locale en-US]
    quadcam-intel install [--locale en-US]
    """

var args = Array(CommandLine.arguments.dropFirst())
var locale = "en-US"
if let i = args.firstIndex(of: "--locale") {
    guard i + 1 < args.count else { fail("usage", "--locale needs a value.\n\(usage)", .usage) }
    locale = args[i + 1]
    args.removeSubrange(i...(i + 1))
}
guard let command = args.first else { fail("usage", usage, .usage) }
let rest = Array(args.dropFirst())

switch command {
case "caps":
    await caps(locale: locale)
case "asr":
    if rest.isEmpty { fail("usage", "Name at least one audio file.\n\(usage)", .usage) }
    await asr(files: rest, locale: locale)
case "install":
    await install(locale: locale)
case "--version", "version":
    print("quadcam-intel \(protocolVersion)")
default:
    fail("usage", "Unknown command \(command).\n\(usage)", .usage)
}
