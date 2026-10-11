import AVFoundation
import Foundation
import Speech

/// One feature as `caps` reports it.
struct Feature: Encodable {
    let name: String
    let available: Bool
    /// The oldest macOS the feature runs on.
    let minOS: String
    /// Why it is unavailable, when it is.
    var reason: String?
    /// The locale it was asked about, and its assets: installed, supported (can be
    /// installed), downloading, or unsupported.
    var locale: String?
    var assets: String?

    enum CodingKeys: String, CodingKey {
        case name, available, minOS = "min_os", reason, locale, assets
    }
}

struct Caps: Encodable {
    let kind = "caps"
    let helper = protocolVersion
    let os = osVersion()
    let features: [Feature]
}

func caps(locale: String) async {
    var asr = Feature(name: "asr", available: false, minOS: "26.0")
    if #available(macOS 26.0, *) {
        await SpeechCheck.describe(&asr, locale: locale)
    } else {
        asr.reason = "Speech recognition on this Mac needs macOS 26 or later."
    }
    emit(Caps(features: [asr, videoFeature()]))
}

struct Word: Encodable {
    let text: String
    let start: Double
    let end: Double
}

struct Heard: Encodable {
    let kind = "asr"
    let file: String
    var text: String?
    /// The mean of the words' confidence, 0-1, when the recognizer gave one.
    var confidence: Double?
    var words: [Word]?
    var ms: Int?
    var error: String?
}

struct Installed: Encodable {
    let kind = "install"
    let locale: String
    /// installed (downloaded now) or already (nothing to download).
    let status: String
}

func asr(files: [String], locale: String) async {
    guard #available(macOS 26.0, *) else {
        fail("unavailable", "Speech recognition on this Mac needs macOS 26 or later.", .unavailable)
    }
    let found = await SpeechCheck.ready(locale: locale)
    for f in files {
        emit(await SpeechCheck.transcribe(file: f, locale: found))
    }
}

func install(locale: String) async {
    guard #available(macOS 26.0, *) else {
        fail("unavailable", "Speech recognition on this Mac needs macOS 26 or later.", .unavailable)
    }
    await SpeechCheck.install(locale: locale)
}

@available(macOS 26.0, *)
enum SpeechCheck {
    static func transcriber(_ locale: Locale) -> SpeechTranscriber {
        SpeechTranscriber(
            locale: locale, transcriptionOptions: [], reportingOptions: [],
            attributeOptions: [.audioTimeRange, .transcriptionConfidence])
    }

    /// The assets' status for this helper. A locale the system already holds reads as
    /// `supported` until this process asks for it; an installation request of nil means
    /// nothing is left to download, so it counts as installed. Downloads nothing.
    static func status(_ locale: Locale) async -> AssetInventory.Status {
        let t = transcriber(locale)
        let s = await AssetInventory.status(forModules: [t])
        if s == .supported, (try? await AssetInventory.assetInstallationRequest(supporting: [t])) == .some(nil) {
            return .installed
        }
        return s
    }

    static func assetText(_ s: AssetInventory.Status) -> String {
        switch s {
        case .installed: return "installed"
        case .downloading: return "downloading"
        case .supported: return "supported"
        default: return "unsupported"
        }
    }

    static func describe(_ f: inout Feature, locale: String) async {
        f.locale = locale
        guard SpeechTranscriber.isAvailable else {
            f.reason = "Speech recognition is not available on this Mac."
            return
        }
        guard let found = await SpeechTranscriber.supportedLocale(equivalentTo: Locale(identifier: locale))
        else {
            f.assets = "unsupported"
            f.reason = "Speech recognition does not support \(locale)."
            return
        }
        let status = await status(found)
        f.assets = assetText(status)
        if status == .installed {
            f = Feature(
                name: f.name, available: true, minOS: f.minOS, reason: nil, locale: locale,
                assets: f.assets)
        } else {
            f.reason = "The speech assets for \(locale) are not installed: run quadcam-intel install --locale \(locale)."
        }
    }

    /// The supported locale, or an exit with why the check cannot run.
    static func ready(locale: String) async -> Locale {
        guard SpeechTranscriber.isAvailable else {
            fail("unavailable", "Speech recognition is not available on this Mac.", .unavailable)
        }
        guard let found = await SpeechTranscriber.supportedLocale(equivalentTo: Locale(identifier: locale))
        else {
            fail("unsupported_locale", "Speech recognition does not support \(locale).", .unavailable)
        }
        let status = await status(found)
        if status != .installed {
            fail(
                "assets_not_installed",
                "The speech assets for \(locale) are not installed (\(assetText(status))): run quadcam-intel install --locale \(locale).",
                .unavailable)
        }
        return found
    }

    static func transcribe(file: String, locale: Locale) async -> Heard {
        let start = Date()
        var out = Heard(file: file)
        do {
            let audio = try AVAudioFile(forReading: URL(fileURLWithPath: file))
            let t = transcriber(locale)
            let analyzer = SpeechAnalyzer(modules: [t])
            async let collected: [SpeechTranscriber.Result] = t.results.reduce(into: []) { $0.append($1) }
            if let last = try await analyzer.analyzeSequence(from: audio) {
                try await analyzer.finalizeAndFinish(through: last)
            } else {
                await analyzer.cancelAndFinishNow()
            }
            var text = ""
            var words: [Word] = []
            var conf: [Double] = []
            for r in try await collected {
                text += String(r.text.characters)
                for run in r.text.runs {
                    let piece = String(r.text[run.range].characters)
                        .trimmingCharacters(in: .whitespacesAndNewlines)
                    if let c = run[AttributeScopes.SpeechAttributes.ConfidenceAttribute.self] {
                        conf.append(c)
                    }
                    if !piece.isEmpty,
                        let tr = run[AttributeScopes.SpeechAttributes.TimeRangeAttribute.self]
                    {
                        words.append(
                            Word(
                                text: piece, start: round3(tr.start.seconds),
                                end: round3(tr.end.seconds)))
                    }
                }
            }
            out.text = text.trimmingCharacters(in: .whitespacesAndNewlines)
            out.words = words
            out.confidence = conf.isEmpty ? nil : round3(conf.reduce(0, +) / Double(conf.count))
        } catch {
            out.error = "\(error.localizedDescription)"
        }
        out.ms = Int(Date().timeIntervalSince(start) * 1000)
        return out
    }

    static func install(locale: String) async {
        guard SpeechTranscriber.isAvailable else {
            fail("unavailable", "Speech recognition is not available on this Mac.", .unavailable)
        }
        guard let found = await SpeechTranscriber.supportedLocale(equivalentTo: Locale(identifier: locale))
        else {
            fail("unsupported_locale", "Speech recognition does not support \(locale).", .unavailable)
        }
        do {
            if let req = try await AssetInventory.assetInstallationRequest(supporting: [transcriber(found)]) {
                try await req.downloadAndInstall()
                emit(Installed(locale: locale, status: "installed"))
            } else {
                emit(Installed(locale: locale, status: "already"))
            }
        } catch {
            fail("install_failed", "The speech assets for \(locale) did not install: \(error.localizedDescription)", .failed)
        }
    }
}

func round3(_ x: Double) -> Double { (x * 1000).rounded() / 1000 }
