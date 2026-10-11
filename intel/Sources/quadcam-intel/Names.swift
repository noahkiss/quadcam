import CoreGraphics
import Foundation
import FoundationModels
import ImageIO

// `suggest`: clip names, keywords and notes, and a flying day's summary, from Apple's
// on-device language model (Foundation Models, macOS 26+; image input macOS 27+). QuadCam
// owns the facts it sends; this file owns the instructions and the answers' shapes.

/// The request QuadCam sends, as JSON in a file or on stdin.
struct SuggestRequest: Decodable {
    /// `names` (the default) or `summary`.
    var task: String?
    /// Names the person gave earlier clips, for their style.
    var examples: [String]?
    var clips: [ClipFacts]?
    /// The day's facts, for `summary`.
    var facts: String?
    /// False keeps the frames out of the prompt even where the model takes images.
    var images: Bool?
}

struct ClipFacts: Decodable {
    let id: Int
    /// What QuadCam knows about the clip, one fact per line.
    let facts: String
    /// Still frames of the clip (JPEG or PNG). The first `maxFrames` are used, on macOS 27+.
    var frames: [String]?
}

/// Frames per clip: the model's context is small (4096 tokens).
let maxFrames = 2

struct Suggestion: Encodable {
    let kind = "suggestion"
    let id: Int
    var name: String?
    var keywords: [String]?
    var note: String?
    /// The frames the prompt held.
    var images = 0
    var ms = 0
    var error: String?
}

struct GroupName: Encodable {
    let kind = "group"
    let name: String
    let ms: Int
}

struct Summary: Encodable {
    let kind = "summary"
    let text: String
    let ms: Int
}

/// The `suggest` feature as `caps` reports it, with whether the model takes images.
func namesFeature() -> Feature {
    var f = Feature(name: "suggest", available: false, minOS: "26.0")
    guard #available(macOS 26.0, *) else {
        f.reason = "On-device suggestions need macOS 26 or later."
        return f
    }
    if let why = Names.unavailable() {
        f.reason = why
        return f
    }
    var on = Feature(name: "suggest", available: true, minOS: "26.0")
    on.images = Names.takesImages()
    return on
}

func suggest(path: String?) async {
    let data: Data
    do {
        if let path, path != "-" {
            data = try Data(contentsOf: URL(fileURLWithPath: path))
        } else {
            data = FileHandle.standardInput.readDataToEndOfFile()
        }
    } catch {
        fail("usage", "The request could not be read: \(error.localizedDescription)", .usage)
    }
    let req: SuggestRequest
    do {
        req = try JSONDecoder().decode(SuggestRequest.self, from: data)
    } catch {
        fail("usage", "The request is not valid JSON: \(error.localizedDescription)", .usage)
    }
    guard #available(macOS 26.0, *) else {
        fail("unavailable", "On-device suggestions need macOS 26 or later.", .unavailable)
    }
    if let why = Names.unavailable() {
        fail("unavailable", why, .unavailable)
    }
    switch req.task ?? "names" {
    case "names":
        await Names.names(req)
    case "summary":
        await Names.summary(req)
    case let t:
        fail("usage", "Unknown task \(t): names or summary.", .usage)
    }
}

@available(macOS 26.0, *)
@Generable
struct ClipIdea {
    @Guide(description: "A short name for the clip: 2 to 4 lowercase words, no date, no number, no file extension")
    var name: String
    @Guide(description: "Search keywords: the setting, the scenery, the flying style", .maximumCount(5))
    var keywords: [String]
    @Guide(description: "One short sentence on what happens in the clip")
    var note: String
}

@available(macOS 26.0, *)
@Generable
struct SessionIdea {
    @Guide(description: "One name for the whole flying session: 2 to 4 lowercase words, no date, no number")
    var name: String
}

@available(macOS 26.0, *)
@Generable
struct DayIdea {
    @Guide(description: "Two or three plain sentences about the flying day")
    var text: String
}

@available(macOS 26.0, *)
enum Names {
    static let model = SystemLanguageModel.default

    /// Why the model cannot answer here, or nil when it can.
    static func unavailable() -> String? {
        switch model.availability {
        case .available:
            return nil
        case .unavailable(.deviceNotEligible):
            return "This Mac cannot run Apple's on-device model."
        case .unavailable(.appleIntelligenceNotEnabled):
            return "Apple Intelligence is off (System Settings > Apple Intelligence & Siri)."
        case .unavailable(.modelNotReady):
            return "Apple's on-device model is still downloading or not ready."
        case .unavailable:
            return "Apple's on-device model is unavailable."
        }
    }

    /// Whether the model takes images (macOS 27 and its SDK).
    static func takesImages() -> Bool {
        #if compiler(>=6.4)
            if #available(macOS 27.0, *) {
                return model.capabilities.contains(.vision)
            }
        #endif
        return false
    }

    static let clipInstructions = """
        You name short FPV drone video clips. A clip is one battery's flight, filmed from the \
        drone's camera. Use the facts and the frames you are given; do not invent places or \
        people. Ignore text and numbers the goggles draw over the picture (the OSD). Names are \
        plain lowercase words, in the style of the example names when there are any.
        """

    static func examplesText(_ examples: [String]) -> String {
        examples.isEmpty ? "" : "Names the pilot gave earlier clips: " + examples.prefix(8).joined(separator: "; ")
    }

    static func names(_ req: SuggestRequest) async {
        let examples = examplesText(req.examples ?? [])
        let images = (req.images ?? true) && takesImages()
        var named: [String] = []
        for clip in req.clips ?? [] {
            let start = Date()
            var out = Suggestion(id: clip.id)
            do {
                let session = LanguageModelSession(instructions: clipInstructions)
                let frames = images ? (clip.frames ?? []).prefix(maxFrames).compactMap(loadFrame) : []
                out.images = frames.count
                let idea = try await respond(session, examples: examples, facts: clip.facts, frames: frames)
                out.name = tidy(idea.name)
                out.keywords = idea.keywords.map(tidy).filter { !$0.isEmpty }
                out.note = idea.note.trimmingCharacters(in: .whitespacesAndNewlines)
                if let n = out.name, !n.isEmpty { named.append(n) }
            } catch {
                out.error = describe(error)
            }
            out.ms = Int(Date().timeIntervalSince(start) * 1000)
            emit(out)
        }
        // One name for the session, so the clips of one outing read as a set.
        if named.count > 1 {
            let start = Date()
            do {
                let session = LanguageModelSession(instructions: clipInstructions)
                let prompt = """
                    One flying session gave clips with these names: \(named.joined(separator: "; ")).
                    \(examples)
                    Name the whole session in 2 to 4 words: what the clips share, such as the \
                    time of day, the place or the flying style. Do not join the names together.
                    """
                let r = try await session.respond(to: prompt, generating: SessionIdea.self)
                let name = tidy(r.content.name)
                if !name.isEmpty {
                    emit(GroupName(name: name, ms: Int(Date().timeIntervalSince(start) * 1000)))
                }
            } catch {
                // No session name: each clip keeps its own.
            }
        }
    }

    static func respond(
        _ session: LanguageModelSession, examples: String, facts: String, frames: [CGImage]
    ) async throws -> ClipIdea {
        let text = "\(examples)\nFacts about this clip:\n\(facts)\nName this clip."
        #if compiler(>=6.4)
            if #available(macOS 27.0, *), !frames.isEmpty {
                let r = try await session.respond(generating: ClipIdea.self) {
                    text
                    for f in frames {
                        Attachment(f)
                    }
                }
                return r.content
            }
        #endif
        return try await session.respond(to: text, generating: ClipIdea.self).content
    }

    static func summary(_ req: SuggestRequest) async {
        let start = Date()
        guard let facts = req.facts, !facts.isEmpty else {
            fail("usage", "A summary needs facts.", .usage)
        }
        do {
            let session = LanguageModelSession(
                instructions: """
                    You write a short, friendly summary of a day of FPV drone flying for the \
                    pilot's log. Use only the facts given. Plain sentences, no lists, no headings.
                    """)
            let r = try await session.respond(to: "Facts about the day:\n\(facts)", generating: DayIdea.self)
            emit(Summary(text: r.content.text.trimmingCharacters(in: .whitespacesAndNewlines),
                         ms: Int(Date().timeIntervalSince(start) * 1000)))
        } catch {
            fail("failed", describe(error), .failed)
        }
    }

    /// A name or keyword in lowercase words: no quotes, no trailing stop.
    static func tidy(_ s: String) -> String {
        s.lowercased()
            .trimmingCharacters(in: .whitespacesAndNewlines.union(CharacterSet(charactersIn: "\"'.")))
            .split(whereSeparator: \.isWhitespace).joined(separator: " ")
    }

    static func describe(_ error: Error) -> String {
        if let e = error as? LanguageModelSession.GenerationError {
            switch e {
            case .guardrailViolation: return "The model declined (guardrail)."
            case .refusal: return "The model declined."
            case .exceededContextWindowSize: return "The facts were too long for the model."
            default: break
            }
        }
        return error.localizedDescription
    }

    /// A frame with its edges cut off (the OSD sits there), or nil when it cannot be read.
    static func loadFrame(_ path: String) -> CGImage? {
        guard let src = CGImageSourceCreateWithURL(URL(fileURLWithPath: path) as CFURL, nil),
            let img = CGImageSourceCreateImageAtIndex(src, 0, nil)
        else { return nil }
        let w = CGFloat(img.width)
        let h = CGFloat(img.height)
        return img.cropping(to: CGRect(x: w * 0.1, y: h * 0.1, width: w * 0.8, height: h * 0.8)) ?? img
    }
}
