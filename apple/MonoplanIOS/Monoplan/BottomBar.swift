// The floating bar at the foot of every page: a search field and, to
// its right, an add button. Placed as the bottom safe-area inset of
// Browse's navigation stack, so it stays put across pushed pages and
// lists scroll under it. Liquid Glass on iOS 26, material before.
// While the field has focus the add button becomes a cross that
// cancels the search: clears the text and drops the keyboard. Nothing
// else works yet: the field takes text and add is inert until search
// and capture land.

import SwiftUI

struct BottomBar: View {
    @State private var query = ""
    @FocusState private var searching: Bool

    private static let height: CGFloat = 48

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
                if searching {
                    query = ""
                    searching = false
                }
                // Otherwise capture: not built yet.
            } label: {
                Image(systemName: searching ? "xmark" : "square.and.pencil")
                    .font(.title3.weight(.semibold))
                    .contentTransition(.symbolEffect(.replace))
                    .frame(width: Self.height, height: Self.height)
                    .contentShape(Circle())
            }
            .buttonStyle(.plain)
            .glass(in: Circle())
            .accessibilityLabel(searching ? "Cancel search" : "Add")
            .animation(.default, value: searching)
        }
        .padding(.horizontal, 16)
        .padding(.bottom, 8)
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
