// Hand-written Swift over the uniffi-generated API
// (`Generated/monoplan_ffi.swift`). Deliberately minimal — no abstraction
// layer yet. The generated symbols (`Host`, `HostError`, `login(...)`,
// `logout(...)`) are already `public` and re-exported as part of this
// module; nothing here needs ergonomics on top of them yet.

import Foundation
