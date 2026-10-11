import CoreMedia
import Foundation

#if canImport(MediaIntelligence)
    import MediaIntelligence
#endif

/// The `video` feature as `caps` reports it: MediaIntelligence's VideoAnalyzer, macOS 27+.
func videoFeature() -> Feature {
    var f = Feature(name: "video", available: false, minOS: "27.0")
    #if canImport(MediaIntelligence)
        if #available(macOS 27.0, *) {
            f = Feature(name: "video", available: true, minOS: "27.0")
        } else {
            f.reason = "Video analysis on this Mac needs macOS 27 or later."
        }
    #else
        f.reason = "This copy of quadcam-intel was built without the macOS 27 SDK."
    #endif
    return f
}

struct Range: Encodable {
    let start: Double
    let end: Double
}

struct Level: Encodable {
    let start: Double
    let end: Double
    let level: Double
}

/// What the analyzer found in one video file. Each request reports its own error.
struct Analysed: Encodable {
    let kind = "video"
    let file: String
    var highlights: [Range]?
    var levels: [Level]?
    /// Seconds into the file: the frame that best stands for the clip.
    var keyframe: Double?
    var highlightsError: String?
    var keyframeError: String?
    var ms: Int?
    var error: String?

    enum CodingKeys: String, CodingKey {
        case kind, file, highlights, levels, keyframe, ms, error
        case highlightsError = "highlights_error"
        case keyframeError = "keyframe_error"
    }
}

func video(file: String, highlights: Bool, keyframe: Bool) async {
    #if canImport(MediaIntelligence)
        guard #available(macOS 27.0, *) else {
            fail("unavailable", "Video analysis on this Mac needs macOS 27 or later.", .unavailable)
        }
        emit(await VideoCheck.analyse(file: file, highlights: highlights, keyframe: keyframe))
    #else
        fail("unavailable", "This copy of quadcam-intel was built without the macOS 27 SDK.", .unavailable)
    #endif
}

#if canImport(MediaIntelligence)
    @available(macOS 27.0, *)
    enum VideoCheck {
        static func seconds(_ r: CMTimeRange) -> Range {
            Range(start: round3(r.start.seconds), end: round3(r.end.seconds))
        }

        static func message(_ e: Error) -> String {
            (e as? LocalizedError)?.errorDescription ?? "\(e)"
        }

        static func analyse(file: String, highlights: Bool, keyframe: Bool) async -> Analysed {
            let start = Date()
            var out = Analysed(file: file)
            let url = URL(fileURLWithPath: file)
            guard FileManager.default.isReadableFile(atPath: url.path) else {
                out.error = "The file is not readable."
                return out
            }
            let asset = MediaIntelligenceVideoAsset(id: .init(url.lastPathComponent), kind: .url(url))
            let analyzer = VideoAnalyzer.shared
            do {
                if highlights && keyframe {
                    let (h, k) = try await analyzer.analyze(
                        asset, for: HighlightAnalysisRequest(), KeyFrameAnalysisRequest())
                    take(h, &out)
                    take(k, &out)
                } else if highlights {
                    take(try await analyzer.analyze(asset, for: HighlightAnalysisRequest()), &out)
                } else {
                    take(try await analyzer.analyze(asset, for: KeyFrameAnalysisRequest()), &out)
                }
            } catch {
                out.error = message(error)
            }
            out.ms = Int(Date().timeIntervalSince(start) * 1000)
            return out
        }

        static func take(_ r: Result<HighlightAnalysisRequest.Result, Error>, _ out: inout Analysed) {
            switch r {
            case .success(let h):
                out.highlights = h.highlights.map(seconds)
                out.levels = h.levels.map {
                    Level(
                        start: round3($0.timeRange.start.seconds),
                        end: round3($0.timeRange.end.seconds),
                        level: round3(Double($0.level)))
                }
            case .failure(let e):
                out.highlightsError = message(e)
            }
        }

        static func take(_ r: Result<KeyFrameAnalysisRequest.Result, Error>, _ out: inout Analysed) {
            switch r {
            case .success(let k):
                out.keyframe = k.timestamp.isNumeric ? round3(k.timestamp.seconds) : nil
            case .failure(let e):
                out.keyframeError = message(e)
            }
        }
    }
#endif
