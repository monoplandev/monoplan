// The floating bar at the foot of every page: a search field and, to
// its right, an add button. Placed as the bottom safe-area inset of
// Home's navigation stack, so it stays put across pushed pages and
// lists scroll under it. Liquid Glass on iOS 26, material before.
// Home owns the query and the field's focus, and draws the results
// (`SearchView`) while either is set; for as long, the add button is a
// cross that cancels the search: clears the text and drops the
// keyboard. Otherwise add opens the item sheet on a new item
// (`ItemCaptureView`) for the list being shown, the Personal Inbox
// when no list is.

import SwiftUI

struct BottomBar: View {
    @Binding var query: String
    @FocusState.Binding var searching: Bool
    /// The list a capture starts aimed at; nil is the Personal Inbox.
    var captureList: ListRef?
    @Environment(\.captureItem) private var capture

    /// Search is up: the field has focus, or holds a query whose
    /// results are being read with the keyboard down.
    private var active: Bool { searching || !query.isEmpty }

    private static let height: CGFloat = 48
    private static let bottomPadding: CGFloat = 8

    /// The room a scroll view leaves at its foot so its last row
    /// clears the bar: the bar, its padding, and a gap above it.
    static let clearance: CGFloat = height + bottomPadding + 8

    var body: some View {
        HStack(spacing: 10) {
            HStack(spacing: 8) {
                Image(systemName: "magnifyingglass")
                    .foregroundStyle(.secondary)
                TextField("Search", text: $query)
                    .textInputAutocapitalization(.never)
                    .autocorrectionDisabled()
                    .submitLabel(.search)
                    .focused($searching)
            }
            .padding(.horizontal, 16)
            .frame(height: Self.height)
            .glass(in: Capsule())

            // One button in one glass circle, so the glyph swaps in
            // place rather than one control replacing another.
            Button {
                if active {
                    query = ""
                    searching = false
                } else {
                    capture(captureList)
                }
            } label: {
                Image(systemName: active ? "xmark" : "square.and.pencil")
                    .font(.title3.weight(.semibold))
                    .contentTransition(.symbolEffect(.replace))
                    .frame(width: Self.height, height: Self.height)
                    .contentShape(Circle())
            }
            .buttonStyle(.plain)
            .glass(in: Circle())
            .accessibilityLabel(active ? "Cancel search" : "Add")
            .animation(.default, value: active)
        }
        .padding(.horizontal, 16)
        .padding(.bottom, Self.bottomPadding)
    }
}

private extension View {
    /// The bar's surface: Liquid Glass where there is any, else the
    /// material the old tab bar had, with a hairline to hold the edge.
    @ViewBuilder
    func glass(in shape: some Shape) -> some View {
        if #available(iOS 26, *) {
            glassEffect(.regular.interactive(), in: shape)
        } else {
            background(.regularMaterial, in: shape)
                .overlay(shape.stroke(.separator, lineWidth: 0.5))
        }
    }
}
