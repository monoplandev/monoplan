// The sign-in sheet: the macOS Settings sheet's form, sign-in only.
// Account creation and recovery stay on the Mac, the web client and
// the CLI for now. The server is a choice of two: monoplan.app (the
// default) or a self-hosted server, whose URL field shows only when it
// is picked. The device is named by the system (`UIDevice.current.name`,
// "iPhone" without the user-assigned-name entitlement), not asked for.

import SwiftUI
import UIKit

struct SignInView: View {
    @EnvironmentObject private var model: AppModel
    @Environment(\.dismiss) private var dismiss
    @State private var email = ""
    @State private var password = ""
    @State private var selfHosted = false
    /// The self-hosted server's URL; read only while `selfHosted`.
    @State private var server = ""

    static let defaultServer = "https://alpha.monoplan.app"

    private var serverText: String { server.trimmingCharacters(in: .whitespacesAndNewlines) }

    private var canSubmit: Bool {
        !model.signingIn && !email.isEmpty && !password.isEmpty && (!selfHosted || !serverText.isEmpty)
    }

    var body: some View {
        NavigationStack {
            Form {
                Section {
                    TextField("Email", text: $email)
                        .textContentType(.username)
                        .keyboardType(.emailAddress)
                        .textInputAutocapitalization(.never)
                        .autocorrectionDisabled()
                    SecureField("Password", text: $password)
                        .textContentType(.password)
                        .onSubmit { if canSubmit { submit() } }
                }
                Section {
                    Picker("Signing in with", selection: $selfHosted) {
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
                            Text("Sign In")
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
            .navigationTitle("Sign In")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button("Cancel") { dismiss() }
                        .disabled(model.signingIn)
                }
            }
            .onDisappear { model.signInError = nil }
        }
    }

    private func submit() {
        let creds = SignInCredentials(
            server: selfHosted ? Self.normalise(server) : Self.defaultServer,
            email: email.trimmingCharacters(in: .whitespaces),
            password: password, deviceName: UIDevice.current.name)
        model.signIn(creds) { ok in
            if ok { dismiss() }
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
