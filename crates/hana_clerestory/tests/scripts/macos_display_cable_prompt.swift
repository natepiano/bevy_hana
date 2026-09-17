// Prompt the operator to unplug or reconnect one external display, then wait for macOS to report it.
//
// The Samsung C34J79x answers no DDC/CI and sits on no smart plug, so a person pulls its cable. A
// software disable (`CGSConfigureDisplayEnabled`) is no substitute: macOS drops the display from
// winit's monitor list before it moves any window off that display, and `bevy_winit` then despawns
// every window on the departed monitor through the `linked_spawn` `HasWindows` relationship. A
// pulled cable lets macOS relocate the windows first, as the Dell's smart-plug power cut did.
//
// The instruction is spoken with `/usr/bin/say`, which takes no keyboard focus from the probe
// windows, and repeats until the display leaves or returns. A display already in the requested
// state exits at once, so the controller's repeated power-on requests stay silent.
//
// usage: swift macos_display_cable_prompt.swift on|off <vendor-hex> <model-hex> <serial-hex> <spoken-name>

import CoreGraphics
import Foundation

let promptIntervalSeconds: TimeInterval = 15
let changeTimeoutSeconds: TimeInterval = 110

func fail(_ message: String) -> Never {
    FileHandle.standardError.write(Data((message + "\n").utf8))
    exit(1)
}

let arguments = CommandLine.arguments
guard arguments.count == 6, ["on", "off"].contains(arguments[1]),
    let vendor = UInt32(arguments[2], radix: 16),
    let model = UInt32(arguments[3], radix: 16),
    let serial = UInt32(arguments[4], radix: 16)
else {
    fail(
        "usage: macos_display_cable_prompt.swift on|off <vendor-hex> <model-hex> <serial-hex> <spoken-name>"
    )
}
let spokenName = arguments[5]
let wantsOnline = arguments[1] == "on"

func matchingDisplayIsOnline() -> Bool {
    var ids = [CGDirectDisplayID](repeating: 0, count: 32)
    var count: UInt32 = 0
    guard CGGetOnlineDisplayList(32, &ids, &count) == .success else {
        fail("CGGetOnlineDisplayList failed")
    }
    return ids.prefix(Int(count)).contains {
        CGDisplayVendorNumber($0) == vendor && CGDisplayModelNumber($0) == model
            && CGDisplaySerialNumber($0) == serial
    }
}

/// Start speaking without waiting, so the display is still polled while the sentence plays.
func speak(_ sentence: String) {
    let say = Process()
    say.executableURL = URL(fileURLWithPath: "/usr/bin/say")
    say.arguments = [sentence]
    do {
        try say.run()
    } catch {
        fail("cannot run /usr/bin/say: \(error)")
    }
}

if matchingDisplayIsOnline() == wantsOnline { exit(0) }

let instruction = wantsOnline ? "Plug the \(spokenName) back in." : "Unplug the \(spokenName)."
FileHandle.standardError.write(Data((instruction + "\n").utf8))
let deadline = Date().addingTimeInterval(changeTimeoutSeconds)
var nextPrompt = Date()
while Date() < deadline {
    if matchingDisplayIsOnline() == wantsOnline { exit(0) }
    if Date() >= nextPrompt {
        speak(instruction)
        nextPrompt = Date().addingTimeInterval(promptIntervalSeconds)
    }
    Thread.sleep(forTimeInterval: 0.1)
}
fail("the \(spokenName) did not \(wantsOnline ? "return" : "leave") within \(Int(changeTimeoutSeconds))s")
