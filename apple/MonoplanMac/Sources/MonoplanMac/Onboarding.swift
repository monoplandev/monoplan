// The account window: signing in or creating an account, as its own
// small window of steps rather than a sheet over Settings. Every way in
// (the tray's "Sign In…", the main window's toolbar, Settings ›
// Account) opens this one window.
//
// Steps: a welcome that offers the two paths, the form (email and
// password), then for a new account the recovery code, shown once, and
// a closing page. The server is part of the account, not a preference,
// so it is set here, behind the gear in the corner since almost nobody
// changes it; one value serves both paths. The device name is not
// asked: it is this Mac's name, and Settings › Devices renames it.
//
// Creating an account adopts the local workspace the app has been
// running (`spec/auth.md` §Device pairing); signing in to an existing
// account replaces that workspace, which the form says.

import AppKit
import SwiftUI
import MonoplanCore
import MonoplanMacKit

final class OnboardingWindowController: NSWindowController, NSWindowDelegate {
    private let model: OnboardingModel

    /// `openApp` is the closing page's button: the main window.
    init(
        location: ProfileLocation,
        signIn: @escaping (SignInCredentials, @escaping (Error?) -> Void) -> Void,
        signUp: @escaping (SignInCredentials, @escaping (Result<SignupOutcome, Error>) -> Void) -> Void,
        openApp: @escaping () -> Void
    ) {
        model = OnboardingModel(location: location, signIn: signIn, signUp: signUp)
        let host = NSHostingController(rootView: OnboardingView(model: model))
        let window = NSWindow(contentViewController: host)
        window.styleMask = [.titled, .closable, .fullSizeContentView]
        window.title = String(localized: "Sign In")
        window.titleVisibility = .hidden
        window.titlebarAppearsTransparent = true
        window.isMovableByWindowBackground = true
        window.isReleasedWhenClosed = false
        window.setContentSize(OnboardingView.size)
        window.center()
        super.init(window: window)
        window.delegate = self
        model.finish = { [weak self] in
            self?.close()
            openApp()
        }
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    /// Bring the window forward; a closed one starts over at the
    /// welcome, an open one stays where it is.
    func show() {
        if window?.isVisible != true {
            model.reset()
            window?.center()
        }
        NSApp.activate(ignoringOtherApps: true)
        showWindow(nil)
        window?.makeKeyAndOrderFront(nil)
    }

    /// Not mid-request, and not past the recovery code: the only way
    /// on from it is to confirm the words are written down.
    func windowShouldClose(_ sender: NSWindow) -> Bool {
        !model.busy && !model.showingRecoveryCode
    }

    /// Nothing typed outlives the window.
    func windowWillClose(_ notification: Notification) {
        model.reset()
    }
}

final class OnboardingModel: ObservableObject {
    enum Mode { case signIn, signUp }

    enum Step: Equatable {
        case welcome
        case form
        /// A signup's recovery code, held only while its page shows.
        case recoveryCode(String)
        case done
    }

    static let defaultServer = "https://alpha.monoplan.app"

    @Published var step: Step = .welcome
    @Published var mode: Mode = .signIn
    @Published var email = ""
    @Published var password = ""
    /// Behind the gear: Monoplan's own server, or a custom one.
    @Published var customServer = false
    /// The custom address as typed; kept while Monoplan's is selected,
    /// so switching back and forth loses nothing. Normalised on use.
    @Published var customAddress = ""
    @Published var busy = false
    @Published var error: String?

    /// The closing page's button; set by the window controller.
    var finish: () -> Void = {}

    private let location: ProfileLocation
    private let signInAction: (SignInCredentials, @escaping (Error?) -> Void) -> Void
    private let signUpAction: (SignInCredentials, @escaping (Result<SignupOutcome, Error>) -> Void) -> Void

    init(
        location: ProfileLocation,
        signIn: @escaping (SignInCredentials, @escaping (Error?) -> Void) -> Void,
        signUp: @escaping (SignInCredentials, @escaping (Result<SignupOutcome, Error>) -> Void) -> Void
    ) {
        self.location = location
        self.signInAction = signIn
        self.signUpAction = signUp
    }

    var showingRecoveryCode: Bool {
        if case .recoveryCode = step { return true }
        return false
    }

    /// Whether signing in would replace a local-only workspace (the
    /// form warns; creating an account keeps it instead).
    var hasLocalWorkspace: Bool { location.hasProfile && !location.hasAccount }

    /// The server both paths talk to.
    var server: String { customServer ? Self.normalise(customAddress) : Self.defaultServer }

    var usesDefaultServer: Bool { server == Self.defaultServer }

    var canSubmit: Bool {
        !busy && !email.trimmingCharacters(in: .whitespaces).isEmpty && !password.isEmpty
            && !(customServer && customAddress.trimmingCharacters(in: .whitespaces).isEmpty)
    }

    func choose(_ mode: Mode) {
        self.mode = mode
        error = nil
        step = .form
    }

    func back() {
        error = nil
        password = ""
        step = .welcome
    }

    /// The password goes straight to Rust and is dropped here as soon
    /// as the request succeeds.
    func submit() {
        guard canSubmit else { return }
        let creds = SignInCredentials(
            server: server, email: email.trimmingCharacters(in: .whitespaces),
            password: password)
        busy = true
        error = nil
        switch mode {
        case .signIn:
            signInAction(creds) { [weak self] error in
                guard let self else { return }
                self.busy = false
                if let error {
                    self.error = error.localizedDescription
                } else {
                    self.password = ""
                    self.step = .done
                }
            }
        case .signUp:
            signUpAction(creds) { [weak self] result in
                guard let self else { return }
                self.busy = false
                switch result {
                case .success(let outcome):
                    self.password = ""
                    self.step = .recoveryCode(outcome.recoveryCode)
                case .failure(let error):
                    self.error = error.localizedDescription
                }
            }
        }
    }

    func reset() {
        step = .welcome
        mode = .signIn
        email = ""
        password = ""
        customServer = false
        customAddress = ""
        busy = false
        error = nil
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

struct OnboardingView: View {
    static let size = NSSize(width: 420, height: 540)

    @ObservedObject var model: OnboardingModel
    @State private var showAdvanced = false

    /// The gear is there while the server can still matter.
    private var offersAdvanced: Bool {
        model.step == .welcome || model.step == .form
    }

    var body: some View {
        ZStack(alignment: .topTrailing) {
            Group {
                switch model.step {
                case .welcome: WelcomePage(model: model)
                case .form: FormPage(model: model)
                case .recoveryCode(let code): RecoveryCodePage(code: code) { model.step = .done }
                case .done: DonePage(model: model)
                }
            }
            .padding(.horizontal, 44)
            .padding(.top, 52)
            .padding(.bottom, 32)
            .frame(maxWidth: .infinity, maxHeight: .infinity)

            if offersAdvanced {
                Button { showAdvanced.toggle() } label: {
                    Image(systemName: "gearshape")
                        .font(.system(size: 14))
                        .foregroundStyle(.secondary)
                        .frame(width: 28, height: 28)
                        .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .help("Advanced")
                .accessibilityLabel("Advanced")
                .disabled(model.busy)
                .popover(isPresented: $showAdvanced, arrowEdge: .bottom) {
                    AdvancedPopover(model: model)
                }
                .padding(.top, 8)
                .padding(.trailing, 10)
            }
        }
        .frame(width: Self.size.width, height: Self.size.height)
        .ignoresSafeArea()
        .animation(.easeInOut(duration: 0.2), value: model.step)
    }
}

/// The server, for both paths.
private struct AdvancedPopover: View {
    @ObservedObject var model: OnboardingModel
    @FocusState private var addressFocused: Bool

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            Text("Advanced").font(.headline)
            Text("Server").font(.subheadline).foregroundStyle(.secondary)
            Picker("Server", selection: $model.customServer) {
                Text(verbatim: "Monoplan").tag(false)
                Text("Custom").tag(true)
            }
            .pickerStyle(.segmented)
            .labelsHidden()
            if model.customServer {
                TextField("Server", text: $model.customAddress, prompt: Text(verbatim: "https://example.com"))
                    .textFieldStyle(.roundedBorder)
                    .textContentType(.URL)
                    .autocorrectionDisabled()
                    .labelsHidden()
                    .focused($addressFocused)
            }
            Text("The server your account lives on. Leave it alone unless you host your own.")
                .font(.footnote).foregroundStyle(.secondary)
                .fixedSize(horizontal: false, vertical: true)
        }
        .padding(16)
        .frame(width: 300)
        .onChange(of: model.customServer) { _, custom in
            if custom { addressFocused = true }
        }
    }
}

/// The icon, a heading and a line under it: the top of every page.
private struct PageHeader: View {
    let title: String
    let subtitle: String
    var symbol: String?

    var body: some View {
        VStack(spacing: 10) {
            if let symbol {
                Image(systemName: symbol)
                    .font(.system(size: 52))
                    .foregroundStyle(Color.accentColor)
                    .frame(height: 72)
            } else {
                Image(nsImage: NSApp.applicationIconImage)
                    .resizable()
                    .frame(width: 72, height: 72)
                    .accessibilityHidden(true)
            }
            Text(verbatim: title)
                .font(.title2.weight(.semibold))
                .multilineTextAlignment(.center)
            Text(verbatim: subtitle)
                .foregroundStyle(.secondary)
                .multilineTextAlignment(.center)
                .fixedSize(horizontal: false, vertical: true)
        }
    }
}

private struct WelcomePage: View {
    @ObservedObject var model: OnboardingModel

    var body: some View {
        VStack(spacing: 0) {
            Spacer()
            PageHeader(
                title: String(localized: "Welcome to Monoplan"),
                subtitle: String(localized: "Keep your lists in sync across your devices, end-to-end encrypted."))
            Spacer()
            VStack(spacing: 10) {
                Button { model.choose(.signUp) } label: {
                    Text("Create Account").frame(maxWidth: .infinity)
                }
                .buttonStyle(.borderedProminent)
                .keyboardShortcut(.defaultAction)
                Button { model.choose(.signIn) } label: {
                    Text("Sign In").frame(maxWidth: .infinity)
                }
            }
            .controlSize(.large)
            ServerNote(model: model)
        }
    }
}

/// Which server, said only when it is not the usual one.
private struct ServerNote: View {
    @ObservedObject var model: OnboardingModel

    var body: some View {
        Group {
            if model.usesDefaultServer {
                Text(verbatim: " ")
            } else {
                Text("Using server \(model.server)")
                    .lineLimit(1)
                    .truncationMode(.middle)
            }
        }
        .font(.footnote)
        .foregroundStyle(.secondary)
        .padding(.top, 12)
    }
}

private struct FormPage: View {
    private enum Field { case email, password }

    @ObservedObject var model: OnboardingModel
    @FocusState private var focus: Field?

    private var signingUp: Bool { model.mode == .signUp }

    var body: some View {
        VStack(spacing: 0) {
            PageHeader(
                title: signingUp ? String(localized: "Create your account") : String(localized: "Sign in to Monoplan"),
                subtitle: signingUp
                    ? String(localized: "What is on this Mac becomes your account's and starts syncing.")
                    : String(localized: "Enter the email and password of your account."))
            VStack(spacing: 10) {
                TextField("Email", text: $model.email)
                    .textContentType(.username)
                    .autocorrectionDisabled()
                    .focused($focus, equals: .email)
                    .onSubmit { focus = .password }
                    .modifier(OnboardingField())
                SecureField("Password", text: $model.password)
                    .textContentType(signingUp ? .newPassword : .password)
                    .focused($focus, equals: .password)
                    .onSubmit { model.submit() }
                    .modifier(OnboardingField())
            }
            .disabled(model.busy)
            .padding(.top, 24)

            Group {
                if signingUp {
                    Text("You will get a 12-word recovery code. It is the only way back in if you forget your password: Monoplan cannot reset it for you.")
                } else if model.hasLocalWorkspace {
                    Text("Signing in replaces the local workspace on this Mac with the account's. Create an account instead to keep it.")
                }
            }
            .font(.footnote).foregroundStyle(.secondary)
            .multilineTextAlignment(.center)
            .fixedSize(horizontal: false, vertical: true)
            .padding(.top, 14)

            if let error = model.error {
                Text(verbatim: error)
                    .font(.footnote).foregroundStyle(.red)
                    .multilineTextAlignment(.center)
                    .fixedSize(horizontal: false, vertical: true)
                    .padding(.top, 10)
            }
            Spacer(minLength: 12)
            VStack(spacing: 10) {
                Button { model.submit() } label: {
                    HStack(spacing: 8) {
                        if model.busy { ProgressView().controlSize(.small) }
                        if signingUp { Text("Create Account") } else { Text("Sign In") }
                    }
                    .frame(maxWidth: .infinity)
                }
                .buttonStyle(.borderedProminent)
                .keyboardShortcut(.defaultAction)
                .disabled(!model.canSubmit)
                Button { model.back() } label: {
                    Text("Back").frame(maxWidth: .infinity)
                }
                .keyboardShortcut(.cancelAction)
                .disabled(model.busy)
            }
            .controlSize(.large)
            ServerNote(model: model)
        }
        .onAppear { focus = model.email.isEmpty ? .email : .password }
    }
}

/// A roomier text box than the stock bezel, for a form of two fields.
private struct OnboardingField: ViewModifier {
    func body(content: Content) -> some View {
        content
            .textFieldStyle(.plain)
            .font(.system(size: 14))
            .padding(.horizontal, 10)
            .padding(.vertical, 8)
            .background(Color(nsColor: .textBackgroundColor), in: RoundedRectangle(cornerRadius: 8))
            .overlay(RoundedRectangle(cornerRadius: 8).strokeBorder(Color(nsColor: .separatorColor)))
    }
}

/// The one time the recovery code is ever shown (the web client's
/// pane, `Login.tsx`): the twelve words, Copy, and a checkbox that
/// gates Continue, so the only way on is to confirm they are written
/// down. The window does not close around it.
private struct RecoveryCodePage: View {
    let code: String
    let onContinue: () -> Void
    @State private var saved = false
    @State private var copied = false

    private var words: [String] { code.split(separator: " ").map(String.init) }

    var body: some View {
        VStack(spacing: 0) {
            PageHeader(
                title: String(localized: "Your recovery code"),
                subtitle: String(localized: "Write these 12 words down and keep them somewhere safe. They are shown only once and are the only way to reset a forgotten password."),
                symbol: "key.fill")
            LazyVGrid(columns: Array(repeating: GridItem(.flexible(), alignment: .leading), count: 3), spacing: 8) {
                ForEach(Array(words.enumerated()), id: \.offset) { i, word in
                    HStack(spacing: 6) {
                        Text(verbatim: "\(i + 1).").foregroundStyle(.secondary).monospacedDigit()
                        Text(verbatim: word).textSelection(.enabled)
                    }
                }
            }
            .padding(12)
            .background(Color(nsColor: .textBackgroundColor), in: RoundedRectangle(cornerRadius: 8))
            .overlay(RoundedRectangle(cornerRadius: 8).strokeBorder(Color(nsColor: .separatorColor)))
            .padding(.top, 20)
            Button {
                NSPasteboard.general.clearContents()
                NSPasteboard.general.setString(code, forType: .string)
                copied = true
            } label: {
                if copied { Text("Copied") } else { Text("Copy") }
            }
            .padding(.top, 10)
            Spacer(minLength: 12)
            Toggle("I have written down these 12 words", isOn: $saved)
                .toggleStyle(.checkbox)
            Button { onContinue() } label: {
                Text("Continue").frame(maxWidth: .infinity)
            }
            .buttonStyle(.borderedProminent)
            .controlSize(.large)
            .keyboardShortcut(.defaultAction)
            .disabled(!saved)
            .padding(.top, 14)
        }
    }
}

private struct DonePage: View {
    @ObservedObject var model: OnboardingModel

    var body: some View {
        VStack(spacing: 0) {
            Spacer()
            PageHeader(
                title: String(localized: "You’re all set"),
                subtitle: String(localized: "This Mac now syncs with your account."),
                symbol: "checkmark.circle.fill")
            Spacer()
            Button { model.finish() } label: {
                Text("Start using Monoplan").frame(maxWidth: .infinity)
            }
            .buttonStyle(.borderedProminent)
            .controlSize(.large)
            .keyboardShortcut(.defaultAction)
        }
    }
}
