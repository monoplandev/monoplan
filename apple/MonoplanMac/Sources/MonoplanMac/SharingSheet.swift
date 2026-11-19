// Sharing for a shared workspace, the iOS `SharingView` as a sheet on
// the window (spec/sharing-plan.md §Invites, §Member removal), opened
// from the "…" menu on the workspace's sidebar row. Lists the members;
// to the owner it also offers a single-use invite link and the unused
// links issued before. The link is made by the engine
// (`Engine.createInvite`: a fresh secret, the doc's key wrapped under
// it, only the derived id and the wrap sent), so the one shown here is
// the only copy that will ever exist; it is handed on through the share
// menu or copied, and is gone with the sheet. An owner removes a member
// (never themself, never the last owner) by the row's button or its
// right-click menu, confirmed, since removal is forward-only: what the
// member already synced stays with them.
//
// Every call here is a round trip to the server
// (`MainWindowModel.sharing`), off the doc calls' queue. Fresh state
// each time the sheet opens.
//
// Also here: the workspace row's Delete and Leave confirms, as alerts
// on the window like the Bin's (`BinConfirm`).

import AppKit
import MonoplanCore
import SwiftUI

/// The workspace a Sharing sheet is for.
struct SharingTarget: Identifiable {
    let docId: String
    let name: String
    var id: String { docId }
}

struct SharingSheet: View {
    let target: SharingTarget
    @ObservedObject var model: MainWindowModel
    @Environment(\.dismiss) private var dismiss

    /// Nil until the first load lands.
    @State private var members: [WorkspaceMember]?
    /// The owner's unused links; empty for anyone else.
    @State private var invites: [PendingInvite] = []
    @State private var loadError: String?
    @State private var loading = false
    /// The link made in this sheet, if one has been.
    @State private var issued: IssuedInvite?
    @State private var creating = false
    @State private var copied = false
    @State private var revoking: Set<String> = []
    /// The member a removal is being confirmed for.
    @State private var removing: WorkspaceMember?
    @State private var failure: Failure?

    struct Failure {
        let title: String
        let message: String
    }

    private var me: String { model.accountId ?? "" }
    private var isOwner: Bool {
        members?.contains { $0.accountId == me && $0.owner } ?? false
    }
    private var ownerCount: Int { members?.filter(\.owner).count ?? 0 }

    /// Removable by the viewer: they own the workspace, the row is not
    /// theirs (that is leaving, from the sidebar's menu), and it is not
    /// the only owner.
    private func canRemove(_ member: WorkspaceMember) -> Bool {
        isOwner && member.accountId != me && !(member.owner && ownerCount <= 1)
    }

    var body: some View {
        VStack(spacing: 0) {
            Form {
                membersSection
                if members != nil {
                    linkSection
                    if isOwner && !invites.isEmpty { pendingSection }
                }
            }
            .formStyle(.grouped)
            // As tall as what it shows, not a fixed box: the sheet
            // grows as the members and the link load in.
            .fixedSize(horizontal: false, vertical: true)
            Divider()
            HStack {
                Button {
                    Task { await load() }
                } label: {
                    Label("Refresh", systemImage: "arrow.clockwise")
                }
                .disabled(loading)
                Spacer()
                Button("Done") { dismiss() }
                    .keyboardShortcut(.defaultAction)
            }
            .padding(16)
        }
        .frame(width: 460)
        .navigationTitle(Text("Share “\(target.name)”"))
        .task { await load() }
        .confirmationDialog(
            "Remove user?",
            isPresented: Binding(get: { removing != nil }, set: { if !$0 { removing = nil } }),
            presenting: removing
        ) { member in
            Button("Remove \(member.email)", role: .destructive) { remove(member) }
            Button("Cancel", role: .cancel) {}
        } message: { member in
            Text("\(member.email) will stop receiving changes to this workspace. Anything they already synced stays on their devices.")
        }
        .alert(
            failure?.title ?? "",
            isPresented: Binding(get: { failure != nil }, set: { if !$0 { failure = nil } })
        ) {
            Button("OK", role: .cancel) {}
        } message: {
            Text(failure?.message ?? "")
        }
    }

    // MARK: Sections

    private var membersSection: some View {
        Section {
            if let members {
                ForEach(members, id: \.accountId) { member in
                    MemberRow(member: member, isMe: member.accountId == me) {
                        if canRemove(member) {
                            Button {
                                removing = member
                            } label: {
                                Image(systemName: "minus.circle")
                            }
                            .buttonStyle(.borderless)
                            .help(Text("Remove User…"))
                            .accessibilityLabel(Text("Remove User…"))
                        }
                    }
                    .contextMenu {
                        if isOwner && member.accountId != me {
                            Button("Remove User…") { removing = member }
                                .disabled(!canRemove(member))
                        }
                    }
                }
            } else if let loadError {
                VStack(alignment: .leading, spacing: 8) {
                    Text("Could not load sharing details")
                    Text(loadError).font(.footnote).foregroundStyle(.secondary)
                    Button("Try Again") { Task { await load() } }
                }
            } else {
                HStack(spacing: 8) {
                    ProgressView().controlSize(.small)
                    Text("Loading…").foregroundStyle(.secondary)
                }
            }
        } header: {
            if let members {
                Text("Members (\(members.count))")
            } else {
                Text("Members")
            }
        } footer: {
            Text("Anyone who joins can read everything in this workspace, including its history.")
                .font(.footnote)
                .foregroundStyle(.secondary)
        }
    }

    @ViewBuilder
    private var linkSection: some View {
        Section {
            if !isOwner {
                Text("Only the workspace owner can invite people.")
                    .foregroundStyle(.secondary)
            } else if let issued {
                Text(verbatim: issued.link)
                    .font(.footnote.monospaced())
                    .textSelection(.enabled)
                    .lineLimit(3)
                    .truncationMode(.middle)
                HStack {
                    Button {
                        NSPasteboard.general.clearContents()
                        NSPasteboard.general.setString(issued.link, forType: .string)
                        copied = true
                    } label: {
                        Label(copied ? "Copied" : "Copy Link", systemImage: copied ? "checkmark" : "doc.on.doc")
                    }
                    if let url = URL(string: issued.link) {
                        ShareLink(item: url, subject: Text("Join “\(target.name)” on Monoplan")) {
                            Label("Share…", systemImage: "square.and.arrow.up")
                        }
                    }
                }
            } else {
                HStack {
                    Button {
                        createLink()
                    } label: {
                        Label("Create Invite Link", systemImage: "link.badge.plus")
                    }
                    .disabled(creating)
                    if creating { ProgressView().controlSize(.small) }
                }
            }
        } header: {
            Text("Invite Link")
        } footer: {
            if isOwner {
                Group {
                    if let issued {
                        Text("Single use, expires \(date(issued.expiresAt), format: .dateTime.day().month().hour().minute()). It is shown only now, and anyone who has it can join.")
                    } else {
                        Text("Single use, expires in 7 days.")
                    }
                }
                .font(.footnote)
                .foregroundStyle(.secondary)
            }
        }
    }

    private var pendingSection: some View {
        Section("Unused Links") {
            ForEach(invites, id: \.inviteId) { invite in
                HStack {
                    VStack(alignment: .leading, spacing: 2) {
                        Text("Created \(date(invite.createdAt), format: .relative(presentation: .named))")
                        Text("Expires \(date(invite.expiresAt), format: .dateTime.day().month())")
                            .font(.footnote)
                            .foregroundStyle(.secondary)
                    }
                    Spacer()
                    Button("Revoke", role: .destructive) { revoke(invite.inviteId) }
                        .disabled(revoking.contains(invite.inviteId))
                }
            }
        }
    }

    // MARK: Calls

    /// A sharing call as `async`, its errors as their message.
    private func ask<T>(_ body: @escaping (MonoplanCore.Engine) throws -> T) async throws -> T {
        try await withCheckedThrowingContinuation { continuation in
            model.sharing(body) { result in
                continuation.resume(with: result.mapError { EngineFailure($0) })
            }
        }
    }

    /// The members, then, for the owner, the unused links.
    private func load() async {
        let docId = target.docId
        loading = true
        defer { loading = false }
        do {
            let list = try await ask { try $0.workspaceMembers(docId: docId) }
            members = list
            loadError = nil
            model.noteMembers(list, of: docId)
            if isOwner {
                invites = try await ask { try $0.pendingInvites(docId: docId) }
            } else {
                invites = []
            }
        } catch {
            // A list already on screen stays; a refresh that fails says so.
            if members == nil {
                loadError = error.localizedDescription
            } else {
                failure = Failure(
                    title: String(localized: "Could not load sharing details"), message: error.localizedDescription)
            }
        }
    }

    private func createLink() {
        let docId = target.docId
        creating = true
        copied = false
        Task {
            defer { creating = false }
            do {
                issued = try await ask { try $0.createInvite(docId: docId) }
                invites = (try? await ask { try $0.pendingInvites(docId: docId) }) ?? invites
            } catch {
                failure = Failure(
                    title: String(localized: "Could not create the link"), message: error.localizedDescription)
            }
        }
    }

    private func revoke(_ inviteId: String) {
        let docId = target.docId
        revoking.insert(inviteId)
        Task {
            defer { revoking.remove(inviteId) }
            do {
                try await ask { try $0.revokeInvite(docId: docId, inviteId: inviteId) }
                invites.removeAll { $0.inviteId == inviteId }
                // The link made here is that one: it no longer works.
                if issued?.inviteId == inviteId {
                    issued = nil
                    copied = false
                }
            } catch {
                failure = Failure(
                    title: String(localized: "Could not revoke the link"), message: error.localizedDescription)
            }
        }
    }

    private func remove(_ member: WorkspaceMember) {
        let docId = target.docId
        Task {
            do {
                try await ask { try $0.removeMember(docId: docId, accountId: member.accountId) }
                members?.removeAll { $0.accountId == member.accountId }
            } catch {
                failure = Failure(
                    title: String(localized: "Could not remove the user"), message: error.localizedDescription)
                // Whatever the server now thinks is the truth.
                await load()
            }
        }
    }

    private func date(_ millis: Int64) -> Date {
        Date(timeIntervalSince1970: TimeInterval(millis) / 1000)
    }
}

/// One member: their disc, their email, whether they are the viewer
/// and the owner, and the row's trailing control, if any.
private struct MemberRow<Trailing: View>: View {
    let member: WorkspaceMember
    let isMe: Bool
    @ViewBuilder let trailing: () -> Trailing

    var body: some View {
        HStack(spacing: 10) {
            MemberAvatar(accountId: member.accountId, email: member.email)
            Text(verbatim: member.email)
                .lineLimit(1)
                .truncationMode(.middle)
            Spacer(minLength: 8)
            if isMe { Tag(text: String(localized: "You")) }
            if member.owner { Tag(text: String(localized: "Owner")) }
            trailing()
        }
    }

    private struct Tag: View {
        let text: String
        var body: some View {
            Text(verbatim: text)
                .font(.caption)
                .foregroundStyle(.secondary)
                .padding(.horizontal, 6)
                .padding(.vertical, 2)
                .background(.quaternary, in: Capsule())
        }
    }
}

/// A person as a coloured disc with the first letter of their email:
/// the web's `Avatar` (and iOS's `MemberAvatar`), same palette and same
/// pick from the account id, so a member is the same colour on every
/// device.
/// With no email (an assignee who is no longer a member) the disc is a
/// grey dashed "?", as the web's.
struct MemberAvatar: View {
    let accountId: String
    let email: String?
    var side: CGFloat = 24

    private static let colours: [Color] = [
        0xFF928B, 0xFFD9C6, 0xFEC3A6, 0xFEF2AD, 0xE2BDFE,
        0xFCF5C7, 0xFFEE93, 0x9DC4FF, 0xA2FFC2, 0xADF7B6,
    ].map { hex in
        Color(
            red: Double((hex >> 16) & 0xFF) / 255,
            green: Double((hex >> 8) & 0xFF) / 255,
            blue: Double(hex & 0xFF) / 255)
    }

    /// `avatarColour`: the first eight characters of the id as hex, or
    /// a sum of its code points when they are not.
    private var colour: Color {
        let n = UInt32(accountId.prefix(8), radix: 16)
            ?? accountId.unicodeScalars.reduce(UInt32(0)) { $0 &+ $1.value }
        return Self.colours[Int(n % UInt32(Self.colours.count))]
    }

    private var initial: String {
        let trimmed = (email ?? "").trimmingCharacters(in: .whitespaces)
        return trimmed.first.map { String($0).uppercased() } ?? "?"
    }

    var body: some View {
        Text(verbatim: initial)
            // The small disc's letter stays legible.
            .font(.system(size: max(9, side * 0.5), weight: .semibold))
            .foregroundStyle(email == nil ? AnyShapeStyle(.secondary) : AnyShapeStyle(.black.opacity(0.75)))
            .frame(width: side, height: side)
            .background {
                if email == nil {
                    Circle().strokeBorder(.secondary, style: StrokeStyle(lineWidth: 1, dash: [2, 2]))
                } else {
                    Circle().fill(colour)
                }
            }
            .accessibilityHidden(true)
    }
}

/// The workspace row's Delete and Leave, each asked first as a sheet on
/// the window (a bare alert without one). Delete wants the name typed
/// back, as on iOS: it is for good, for every member.
enum WorkspaceConfirm {
    static func delete(name: String, on window: NSWindow?, then confirmed: @escaping () -> Void) {
        DeleteWorkspacePrompt(name: name).run(on: window, then: confirmed)
    }

    /// Rename…: the name in a field; `renamed` gets what it holds,
    /// trimmed and non-blank, on Rename.
    static func rename(name: String, on window: NSWindow?, then renamed: @escaping (String) -> Void) {
        RenameWorkspacePrompt(name: name).run(on: window, then: renamed)
    }

    static func leave(name: String, on window: NSWindow?, then confirmed: @escaping () -> Void) {
        let alert = NSAlert()
        alert.alertStyle = .warning
        alert.messageText = String(localized: "Leave “\(name)”?")
        alert.informativeText = String(localized: "It is removed from your devices and you lose access. The other members keep it; you will need a new invite link to rejoin.")
        alert.addButton(withTitle: String(localized: "Leave")).hasDestructiveAction = true
        alert.addButton(withTitle: String(localized: "Cancel"))
        if let window {
            alert.beginSheetModal(for: window) { if $0 == .alertFirstButtonReturn { confirmed() } }
        } else if alert.runModal() == .alertFirstButtonReturn {
            confirmed()
        }
    }

    static func failed(_ title: String, _ message: String, on window: NSWindow?) {
        let alert = NSAlert()
        alert.alertStyle = .warning
        alert.messageText = title
        alert.informativeText = message
        if let window { alert.beginSheetModal(for: window) } else { alert.runModal() }
    }
}

/// Delete Workspace's confirm: Delete enabled once the field holds the
/// workspace's name. Keeps itself alive until the alert ends (the
/// sheet's handler holds it).
private final class DeleteWorkspacePrompt: NSObject, NSTextFieldDelegate {
    private let name: String
    private let alert = NSAlert()
    private let field = NSTextField(frame: NSRect(x: 0, y: 0, width: 260, height: 24))

    init(name: String) {
        self.name = name
        super.init()
        alert.alertStyle = .warning
        alert.messageText = String(localized: "Delete “\(name)”?")
        alert.informativeText = String(localized: "Its lists and items are removed from every device, and anyone it is shared with loses access. This cannot be undone.")
        field.placeholderString = String(localized: "Type “\(name)” to confirm")
        field.delegate = self
        alert.accessoryView = field
        alert.addButton(withTitle: String(localized: "Delete")).hasDestructiveAction = true
        alert.addButton(withTitle: String(localized: "Cancel"))
        alert.buttons[0].isEnabled = false
    }

    func run(on window: NSWindow?, then confirmed: @escaping () -> Void) {
        alert.window.initialFirstResponder = field
        let finish = { [self] (response: NSApplication.ModalResponse) in
            if response == .alertFirstButtonReturn && matches { confirmed() }
        }
        if let window {
            alert.beginSheetModal(for: window, completionHandler: finish)
        } else {
            finish(alert.runModal())
        }
    }

    private var matches: Bool {
        field.stringValue.trimmingCharacters(in: .whitespacesAndNewlines) == name
    }

    func controlTextDidChange(_ obj: Notification) {
        alert.buttons[0].isEnabled = matches
    }
}


/// Rename Workspace's sheet: a standard alert with the name in a
/// field, selected, Rename enabled while it has a non-blank character.
/// Return in the field is Rename, Escape is Cancel. Keeps itself alive
/// until the alert ends (the sheet's handler holds it).
private final class RenameWorkspacePrompt: NSObject, NSTextFieldDelegate {
    private let alert = NSAlert()
    private let field = NSTextField(frame: NSRect(x: 0, y: 0, width: 260, height: 24))

    init(name: String) {
        super.init()
        alert.messageText = String(localized: "Rename Workspace")
        field.stringValue = name
        field.placeholderString = String(localized: "Name")
        field.delegate = self
        alert.accessoryView = field
        alert.addButton(withTitle: String(localized: "Rename"))
        alert.addButton(withTitle: String(localized: "Cancel"))
        alert.buttons[0].isEnabled = !typed.isEmpty
    }

    func run(on window: NSWindow?, then renamed: @escaping (String) -> Void) {
        alert.window.initialFirstResponder = field
        let finish = { [self] (response: NSApplication.ModalResponse) in
            if response == .alertFirstButtonReturn && !typed.isEmpty { renamed(typed) }
        }
        if let window {
            alert.beginSheetModal(for: window, completionHandler: finish)
        } else {
            finish(alert.runModal())
        }
    }

    private var typed: String {
        field.stringValue.trimmingCharacters(in: .whitespacesAndNewlines)
    }

    func controlTextDidChange(_ obj: Notification) {
        alert.buttons[0].isEnabled = !typed.isEmpty
    }
}
