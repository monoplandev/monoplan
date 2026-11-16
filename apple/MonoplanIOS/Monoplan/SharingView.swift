// Sharing for a shared workspace, the web's `SharingDialog` as a sheet
// (spec/sharing-plan.md §Invites, §Member removal), opened from the
// "..." menu on the workspace's section header on Home. Lists the
// members; to the owner it also offers a single-use invite link and
// the unused links issued before. The link is made by the engine
// (`Engine.createInvite`: a fresh secret, the doc's key wrapped under
// it, only the derived id and the wrap sent), so the one shown here is
// the only copy that will ever exist; it is handed on through the
// share sheet or copied, and is gone with the sheet. Unused links are
// revoked by a swipe or their button. An owner removes a member by a
// swipe or a long press on their row (never themself, never the last
// owner), confirmed, since removal is forward-only: what the member
// already synced stays with them.
//
// Every call here is a round trip to the server (`AppModel.sharing`),
// off the doc calls' queue. Fresh state each time the sheet opens.

import MonoplanCore
import SwiftUI
import UIKit

/// The workspace a Sharing sheet is for.
struct SharingTarget: Identifiable {
    let docId: String
    let name: String
    var id: String { docId }
}

struct SharingView: View {
    let target: SharingTarget
    @EnvironmentObject private var model: AppModel
    @Environment(\.dismiss) private var dismiss

    /// Nil until the first load lands.
    @State private var members: [WorkspaceMember]?
    /// The owner's unused links; empty for anyone else.
    @State private var invites: [PendingInvite] = []
    @State private var loadError: String?
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

    private var me: String { model.session?.accountId ?? "" }
    private var isOwner: Bool {
        members?.contains { $0.accountId == me && $0.owner } ?? false
    }
    private var ownerCount: Int { members?.filter(\.owner).count ?? 0 }

    /// Removable by the viewer: they own the workspace, the row is not
    /// theirs (that is leaving, from Home's menu), and it is not the
    /// only owner.
    private func canRemove(_ member: WorkspaceMember) -> Bool {
        isOwner && member.accountId != me && !(member.owner && ownerCount <= 1)
    }

    var body: some View {
        NavigationStack {
            Form {
                membersSection
                if members != nil {
                    linkSection
                    if isOwner && !invites.isEmpty { pendingSection }
                }
            }
            .navigationTitle("Share “\(target.name)”")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .confirmationAction) {
                    Button("Done") { dismiss() }
                }
            }
            .task { await load() }
            .refreshable { await load() }
            .confirmationDialog(
                "Remove user?",
                isPresented: Binding(get: { removing != nil }, set: { if !$0 { removing = nil } }),
                titleVisibility: .visible,
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
    }

    // MARK: Sections

    private var membersSection: some View {
        Section {
            if let members {
                ForEach(members, id: \.accountId) { member in
                    MemberRow(member: member, isMe: member.accountId == me)
                        .swipeActions {
                            if canRemove(member) {
                                Button("Remove", role: .destructive) { removing = member }
                            }
                        }
                        .contextMenu {
                            if isOwner && member.accountId != me {
                                Button("Remove User…", systemImage: "person.badge.minus", role: .destructive) {
                                    removing = member
                                }
                                .disabled(!canRemove(member))
                            }
                        }
                }
            } else if let loadError {
                VStack(alignment: .leading, spacing: 8) {
                    Text("Could not load sharing details")
                    Text(loadError).font(.footnote).foregroundStyle(.secondary)
                    Button("Try Again") { Task { await load() } }
                        .buttonStyle(.borderless)
                }
            } else {
                HStack(spacing: 8) {
                    ProgressView()
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
        }
    }

    @ViewBuilder
    private var linkSection: some View {
        Section {
            if !isOwner {
                Text("Only the workspace owner can invite people.")
                    .foregroundStyle(.secondary)
            } else if let issued {
                Text(issued.link)
                    .font(.footnote.monospaced())
                    .textSelection(.enabled)
                    .lineLimit(3)
                    .truncationMode(.middle)
                if let url = URL(string: issued.link) {
                    ShareLink(item: url, subject: Text("Join “\(target.name)” on Monoplan")) {
                        Label("Share Link…", systemImage: "square.and.arrow.up")
                    }
                }
                Button {
                    UIPasteboard.general.string = issued.link
                    copied = true
                } label: {
                    Label(copied ? String(localized: "Copied") : String(localized: "Copy Link"), systemImage: copied ? "checkmark" : "doc.on.doc")
                }
            } else {
                Button {
                    createLink()
                } label: {
                    HStack {
                        Label("Create Invite Link", systemImage: "link.badge.plus")
                        if creating {
                            Spacer()
                            ProgressView()
                        }
                    }
                }
                .disabled(creating)
            }
        } header: {
            Text("Invite Link")
        } footer: {
            if isOwner {
                if let issued {
                    Text("Single use, expires \(date(issued.expiresAt), format: .dateTime.day().month().hour().minute()). It is shown only now, and anyone who has it can join.")
                } else {
                    Text("Single use, expires in 7 days.")
                }
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
                        .buttonStyle(.borderless)
                        .disabled(revoking.contains(invite.inviteId))
                }
                .swipeActions {
                    Button("Revoke", role: .destructive) { revoke(invite.inviteId) }
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
            // A list already on screen stays; a pull that fails says so.
            if members == nil {
                loadError = error.localizedDescription
            } else {
                failure = Failure(title: String(localized: "Could not load sharing details"), message: error.localizedDescription)
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
                failure = Failure(title: String(localized: "Could not create the link"), message: error.localizedDescription)
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
                failure = Failure(title: String(localized: "Could not revoke the link"), message: error.localizedDescription)
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
                failure = Failure(title: String(localized: "Could not remove the user"), message: error.localizedDescription)
                // Whatever the server now thinks is the truth.
                await load()
            }
        }
    }

    private func date(_ millis: Int64) -> Date {
        Date(timeIntervalSince1970: TimeInterval(millis) / 1000)
    }
}

/// One member: their disc, their email, and whether they are the
/// viewer and the owner.
private struct MemberRow: View {
    let member: WorkspaceMember
    let isMe: Bool

    var body: some View {
        HStack(spacing: 12) {
            MemberAvatar(accountId: member.accountId, email: member.email)
            Text(member.email)
                .lineLimit(1)
                .truncationMode(.middle)
            Spacer(minLength: 8)
            if isMe { Tag(text: String(localized: "You")) }
            if member.owner { Tag(text: String(localized: "Owner")) }
        }
        .accessibilityElement(children: .combine)
    }

    private struct Tag: View {
        let text: String
        var body: some View {
            Text(text)
                .font(.caption)
                .foregroundStyle(.secondary)
                .padding(.horizontal, 6)
                .padding(.vertical, 2)
                .background(.quaternary, in: Capsule())
        }
    }
}

/// A person as a coloured disc with the first letter of their email:
/// the web's `Avatar`, same palette and same pick from the account id,
/// so a member is the same colour on every device.
struct MemberAvatar: View {
    let accountId: String
    let email: String
    @ScaledMetric(relativeTo: .body) private var side: CGFloat = 28

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
        let trimmed = email.trimmingCharacters(in: .whitespaces)
        return trimmed.first.map { String($0).uppercased() } ?? "?"
    }

    var body: some View {
        Text(initial)
            .font(.system(size: side * 0.5, weight: .semibold))
            .foregroundStyle(.black.opacity(0.75))
            .frame(width: side, height: side)
            .background(colour, in: Circle())
            .accessibilityHidden(true)
    }
}
