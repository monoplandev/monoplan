// The sign-in sheet: the macOS Settings sheet's form, sign-in only.
// Account creation and recovery stay on the Mac, the web client and
// the CLI for now.

import SwiftUI
import UIKit

struct SignInView: View {
    @EnvironmentObject private var model: AppModel
    @Environment(\.dismiss) private var dismiss
    @State private var email = ""
    @State private var password = ""
    @State private var server = SignInView.defaultServer
    @State private var deviceName = UIDevice.current.name

    static let defaultServer = "https://alpha.monoplan.app"

    private var canSubmit: Bool {
        !model.signingIn && !email.isEmpty && !password.isEmpty && !server.isEmpty && !deviceName.isEmpty
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
                Section("Advanced") {
                    TextField("Server", text: $server)
                        .textContentType(.URL)
                        .keyboardType(.URL)
                        .textInputAutocapitalization(.never)
                        .autocorrectionDisabled()
                    TextField("Device name", text: $deviceName)
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
            server: Self.normalise(server), email: email.trimmingCharacters(in: .whitespaces),
            password: password, deviceName: deviceName.trimmingCharacters(in: .whitespaces))
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
