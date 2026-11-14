// The account sheet, as the macOS Settings sheet has it: sign in to an
// account or create one, switched by the control at the top. Creating
// an account adopts the local workspace the app has been running
// (`spec/auth.md` §Device pairing) and then shows the recovery code
// once, in place of the form; signing in replaces that workspace,
// which the form says. Recovery stays on the Mac, the web client and
// the CLI for now. The server is a choice of two: monoplan.app (the
// default) or a self-hosted server, whose URL field shows only when it
// is picked. The device is named by the system (`UIDevice.current.name`,
// "iPhone" without the user-assigned-name entitlement), not asked for.

import SwiftUI
import UIKit

struct SignInView: View {
    enum Mode { case signIn, signUp }

    /// `spec/auth.md`: 10 characters, enforced by every client since
    /// the server never sees the password (`MIN_PASSWORD_LEN` in the CLI).
    static let minPasswordLength = 10

    @State private var mode = Mode.signIn
    /// A signup's recovery code, held only while the sheet shows it.
    @State private var recoveryCode: String?
    @EnvironmentObject private var model: AppModel
    @Environment(\.dismiss) private var dismiss
    @State private var email = ""
    @State private var password = ""
    @State private var selfHosted = false
    /// The self-hosted server's URL; read only while `selfHosted`.
    @State private var server = ""

    static let defaultServer = "https://alpha.monoplan.app"

    private var serverText: String { server.trimmingCharacters(in: .whitespacesAndNewlines) }

    private var passwordOK: Bool {
        mode == .signIn ? !password.isEmpty : password.count >= Self.minPasswordLength
    }

    private var canSubmit: Bool {
        !model.signingIn && !email.isEmpty && passwordOK && (!selfHosted || !serverText.isEmpty)
    }

    private var title: String { mode == .signIn ? "Sign In" : "Create Account" }

    var body: some View {
        NavigationStack {
            if let code = recoveryCode {
                RecoveryCodeView(code: code) {
                    recoveryCode = nil
                    dismiss()
                }
            } else {
                form
            }
        }
        // The recovery code is shown once: the only way on is Continue.
        .interactiveDismissDisabled(recoveryCode != nil || model.signingIn)
        .onDisappear { model.signInError = nil }
    }

    private var form: some View {
        Form {
            Section {
                Picker("", selection: $mode) {
                    Text("Sign In").tag(Mode.signIn)
                    Text("Create Account").tag(Mode.signUp)
                }
                .pickerStyle(.segmented)
                .labelsHidden()
                .listRowBackground(Color.clear)
                .listRowInsets(EdgeInsets())
            }
            Section {
                TextField("Email", text: $email)
                    .textContentType(.username)
                    .keyboardType(.emailAddress)
                    .textInputAutocapitalization(.never)
                    .autocorrectionDisabled()
                SecureField("Password", text: $password)
                    .textContentType(mode == .signUp ? .newPassword : .password)
                    .onSubmit { if canSubmit { submit() } }
            } footer: {
                if mode == .signUp {
                    Text("At least \(Self.minPasswordLength) characters. You will get a 12-word recovery code. It is the only way back in if you forget your password: Monoplan cannot reset it for you.")
                } else if model.session?.localOnly == true {
                    Text("Signing in replaces the local workspace on this iPhone with the account's. Create an account instead to keep it.")
                }
            }
            Section {
                Picker(mode == .signIn ? "Signing in with" : "Signing up with", selection: $selfHosted) {
                    Text("monoplan.app").tag(false)
                    Text("Self-hosted server").tag(true)
                }
                if selfHosted {
                    TextField("Server URL", text: $server)
                        .textContentType(.URL)
                        .keyboardType(.URL)
                        .textInputAutocapitalization(.never)
                        .autocorrectionDisabled()
                }
            }
            if let error = model.signInError {
                Section {
                    Text(error).font(.footnote).foregroundStyle(.red)
                }
            }
            Section {
                Button {
                    submit()
                } label: {
                    HStack {
                        Text(title)
                        if model.signingIn {
                            Spacer()
                            ProgressView()
                        }
                    }
                }
                .disabled(!canSubmit)
            }
        }
        .disabled(model.signingIn)
        .navigationTitle(title)
        .navigationBarTitleDisplayMode(.inline)
        .toolbar {
            ToolbarItem(placement: .cancellationAction) {
                Button("Cancel") { dismiss() }
                    .disabled(model.signingIn)
            }
        }
        .onChange(of: mode) { model.signInError = nil }
    }

    private func submit() {
        let creds = SignInCredentials(
            server: selfHosted ? Self.normalise(server) : Self.defaultServer,
            email: email.trimmingCharacters(in: .whitespaces),
            password: password, deviceName: UIDevice.current.name)
        switch mode {
        case .signIn:
            model.signIn(creds) { ok in
                if ok { dismiss() }
            }
        case .signUp:
            model.signUp(creds) { code in
                if let code { recoveryCode = code }
            }
        }
    }

    /// A bare host becomes https; a trailing slash would double up
    /// against the `/api/…` paths the client appends.
    static func normalise(_ raw: String) -> String {
        var s = raw.trimmingCharacters(in: .whitespacesAndNewlines)
        if !s.contains("://") { s = "https://" + s }
        while s.hasSuffix("/") { s.removeLast() }
        return s
    }
}

/// The one time the recovery code is ever shown (the Mac sheet's pane,
/// the web's `Login.tsx`): the twelve words, Copy, and a toggle that
/// gates Continue, so the only way on is to confirm they are written
/// down. There is no Cancel and the sheet cannot be swiped away.
struct RecoveryCodeView: View {
    let code: String
    let onContinue: () -> Void
    @State private var saved = false
    @State private var copied = false

    private var words: [String] { code.split(separator: " ").map(String.init) }

    var body: some View {
        Form {
            Section {
                LazyVGrid(columns: Array(repeating: GridItem(.flexible(), alignment: .leading), count: 2), spacing: 10) {
                    ForEach(Array(words.enumerated()), id: \.offset) { i, word in
                        HStack(spacing: 6) {
                            Text(verbatim: "\(i + 1).").foregroundStyle(.secondary).monospacedDigit()
                            Text(word)
                        }
                    }
                }
                .accessibilityElement(children: .ignore)
                .accessibilityLabel(code)
                Button(copied ? "Copied" : "Copy") {
                    UIPasteboard.general.string = code
                    copied = true
                }
            } footer: {
                Text("Write these 12 words down and keep them somewhere safe. They are shown only once and are the only way to reset a forgotten password.")
            }
            Section {
                Toggle("I have written down these 12 words", isOn: $saved)
            }
            Section {
                Button("Continue") { onContinue() }
                    .disabled(!saved)
            }
        }
        .navigationTitle("Your Recovery Code")
        .navigationBarTitleDisplayMode(.inline)
    }
}
