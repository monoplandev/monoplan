// The assignee editor, in place of the item's page for as long as it
// is open: a header (cross, "Assignee" over the item's title, tick),
// then one run of full-width rows: "Unassigned", then the workspace's
// members, the current pick ticked (spec/sharing-plan.md "Assignment").
// One assignee, so a pick replaces the last. A tap on a row picks it
// and closes; the cross and the tick both close with nothing changed.
// It takes the sheet over as the list editor does (`ItemListEditor`),
// rather than dropping a system menu from the field.

import MonoplanCore
import SwiftUI

struct ItemAssigneeEditor: View {
    let itemTitle: String
    /// The workspace's members; nil while the list is not known.
    let members: [WorkspaceMember]?
    /// The assignee's account id; nil when unassigned.
    let current: String?
    let pick: (String?) -> Void
    let done: () -> Void

    @ScaledMetric(relativeTo: .body) private var avatarSide: CGFloat = 28

    /// What an assignee shows as: a member's email. One the member list
    /// does not hold is a former member; while the list is not known
    /// there is only the fact that there is one.
    static func name(of accountId: String, in members: [WorkspaceMember]?) -> String {
        if let member = members?.first(where: { $0.accountId == accountId }) { return member.email }
        return members == nil ? String(localized: "Assigned") : String(localized: "A former member")
    }

    /// The current assignee when the member list has no row for them,
    /// so there is always a row to tick.
    private var stranger: String? {
        guard let current, members?.contains(where: { $0.accountId == current }) != true else { return nil }
        return current
    }

    var body: some View {
        ScrollView {
            LazyVStack(spacing: 0) {
                row(nil, text: String(localized: "Unassigned")) {
                    Image(systemName: "person").foregroundStyle(.secondary)
                }
                Divider()
                ForEach(members ?? [], id: \.accountId) { member in
                    row(member.accountId, text: member.email) {
                        MemberAvatar(accountId: member.accountId, email: member.email)
                    }
                    Divider()
                }
                if let stranger {
                    row(stranger, text: Self.name(of: stranger, in: members)) {
                        MemberAvatar(accountId: stranger, email: nil)
                    }
                    Divider()
                }
            }
            .padding(.horizontal)
        }
        .scrollBounceBehavior(.basedOnSize)
        .editorHeader(title: String(localized: "Assignee"), itemTitle: itemTitle, cancel: done, done: done)
    }

    private func row<Icon: View>(_ accountId: String?, text: String, @ViewBuilder icon: () -> Icon) -> some View {
        let ticked = accountId == current
        return Button {
            if !ticked { pick(accountId) }
            done()
        } label: {
            HStack(spacing: 12) {
                icon().frame(width: avatarSide)
                Text(verbatim: text).lineLimit(1)
                Spacer(minLength: 0)
                if ticked {
                    Image(systemName: "checkmark")
                        .font(.body.weight(.semibold))
                        .foregroundStyle(.tint)
                }
            }
            .frame(maxWidth: .infinity, minHeight: 48, alignment: .leading)
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .accessibilityAddTraits(ticked ? .isSelected : [])
    }
}
