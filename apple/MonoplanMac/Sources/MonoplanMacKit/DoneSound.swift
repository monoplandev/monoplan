import AudioToolbox
import Foundation

/// The sound played when an item is ticked done: a device setting, off
/// by default, held in `UserDefaults` under `key` (each app's Settings
/// has the switch). Each app plays it from its model's own tick, so a
/// close arriving by sync or by Undo is silent.
///
/// The clip is `done.wav` in the app's bundle (0.09 s, mono 16-bit PCM,
/// the file the web client plays). It goes out as a system sound: mixed
/// over whatever else is playing with no audio session, silent under
/// the iPhone's ring switch, and on the Mac at the alert volume and
/// under "Play user interface sound effects".
public enum DoneSound {
    public static let key = "doneSound"

    /// Registered on first play; nil when the bundle has no clip (the
    /// SwiftPM test executable).
    private static let sound: SystemSoundID? = {
        guard let url = Bundle.main.url(forResource: "done", withExtension: "wav") else { return nil }
        var id: SystemSoundID = 0
        guard AudioServicesCreateSystemSoundID(url as CFURL, &id) == kAudioServicesNoError else { return nil }
        return id
    }()

    /// Play the tick, if the device has it on.
    public static func play(defaults: UserDefaults = .standard) {
        guard defaults.bool(forKey: key), let sound else { return }
        AudioServicesPlaySystemSound(sound)
    }
}
