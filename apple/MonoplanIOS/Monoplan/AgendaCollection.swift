// The Calendar page's list, as a UIKit collection view (decided
// 2026-11-17). The page opens on Today with a year of days above it,
// and a SwiftUI `List` cannot be put there reliably: it guesses the
// height of every row it has not built, and `ScrollViewProxy.scrollTo`
// is the only way to move it. A collection view's layout says where
// Today's header is, so the page sets the offset itself and corrects it
// on each layout pass until it has appeared.
//
// The list is plain, as the Mac's Calendar is (`AgendaTable.swift`,
// 2026-11-18): no card behind a day's rows. A day's header is a line of
// secondary text at the body's size, its rows sit on the page's own
// ground, and a hairline runs between two rows of a day.
//
// What is overdue (the rows the engine leads Today with, in the overdue
// tone) has a section of its own right above Today, there only while
// something is overdue (`spec/calendar-plan.md` "Overdue"). When it is
// there it is what the page opens on and what "Today" comes back to.
//
// The rows are the SwiftUI ones (`AgendaRowView`), hosted in the cells,
// and so are the headers, which keeps the two on the same margin.

import MonoplanCore
import SwiftUI
import UIKit

struct AgendaCollection: UIViewControllerRepresentable {
    let groups: [AgendaGroup]
    let model: AppModel
    /// The page's count of taps on its "Today" button: a new value
    /// scrolls the list back to Today.
    let todayJumps: Int
    /// The page's `openItem`: the hosted rows do not inherit the
    /// SwiftUI environment, so it is handed to them.
    let openItem: (ItemRef) -> Void
    let refresh: () -> Void

    func makeUIViewController(context: Context) -> AgendaCollectionController {
        let controller = AgendaCollectionController(model: model)
        controller.openItem = openItem
        controller.refresh = refresh
        controller.todayJumps = todayJumps
        controller.show(groups)
        return controller
    }

    func updateUIViewController(_ controller: AgendaCollectionController, context: Context) {
        controller.openItem = openItem
        controller.refresh = refresh
        controller.show(groups)
        if controller.todayJumps != todayJumps {
            controller.todayJumps = todayJumps
            controller.scrollToToday()
        }
    }
}

final class AgendaCollectionController: UICollectionViewController {
    /// A cell: an agenda row, by its day and its key within the day,
    /// or the line Today has when nothing is on it.
    private enum Item: Hashable {
        case row(day: String, key: String)
        case nothingToday
    }

    private typealias Snapshot = NSDiffableDataSourceSnapshot<String, Item>

    private let model: AppModel
    var openItem: (ItemRef) -> Void = { _ in }
    var refresh: () -> Void = {}
    /// The last "Today" tap acted on.
    var todayJumps = 0

    private var dataSource: UICollectionViewDiffableDataSource<String, Item>!
    /// What is shown, as it was handed over.
    private var shown: [AgendaGroup]?
    private var entries: [Item: AgendaEntry] = [:]
    /// Each shown day's header.
    private var titles: [String: String] = [:]
    /// The rows with a line under them: every row of a day but its
    /// last.
    private var divided: Set<Item> = []
    /// The section the page opens on and "Today" comes back to:
    /// Overdue's when it is there (it sits right above Today's), else
    /// Today's.
    private var anchor: String?

    /// The Overdue section's identifier, where a day's date is for the
    /// others.
    private static let overdue = "overdue"

    /// The margin a header and a row keep from the sides of the page.
    private static let margin: CGFloat = 20

    /// The page is still to be put on Today: set by the first rows,
    /// cleared once the page has appeared or a finger is on it. While
    /// it is set every layout pass puts Today's header back at the
    /// top, which is what absorbs the bar insets settling during the
    /// push.
    private var anchoring = false

    init(model: AppModel) {
        self.model = model
        // The grouped appearance, not the plain one: plain pins a
        // day's header under the bar, and a pinned header's frame
        // moves with the scroll, which `todayOffset` reads. The cells
        // clear the grouped ground, so the list looks plain.
        var list = UICollectionLayoutListConfiguration(appearance: .grouped)
        list.headerMode = .supplementary
        list.headerTopPadding = 0
        list.backgroundColor = .systemBackground
        // The line between two rows of a day is the cell's own
        // (`divided`): the system's would also rule a day off above
        // and below.
        list.showsSeparators = false
        super.init(collectionViewLayout: UICollectionViewCompositionalLayout.list(using: list))
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError("not built from a nib") }

    override func viewDidLoad() {
        super.viewDidLoad()
        collectionView.allowsSelection = false
        // The bottom bar sits over every page; the last row scrolls
        // clear of it (`HomeView`'s content margin, which does not
        // reach a UIKit scroll view).
        collectionView.contentInset.bottom = BottomBar.clearance
        collectionView.verticalScrollIndicatorInsets.bottom = BottomBar.clearance

        let pull = UIRefreshControl()
        pull.addAction(UIAction { [weak self, weak pull] _ in
            self?.refresh()
            // The read has no completion to wait on; it is a local
            // one, done well inside this.
            DispatchQueue.main.asyncAfter(deadline: .now() + 0.5) { pull?.endRefreshing() }
        }, for: .valueChanged)
        collectionView.refreshControl = pull

        let cell = UICollectionView.CellRegistration<UICollectionViewListCell, Item> { [unowned self] cell, _, item in
            cell.backgroundConfiguration = .clear()
            guard let row = entries[item] else {
                cell.contentConfiguration = UIHostingConfiguration {
                    Text("Nothing scheduled")
                        .foregroundStyle(.secondary)
                        .frame(maxWidth: .infinity, alignment: .leading)
                }
                .margins(.horizontal, Self.margin)
                .minSize(height: 44)
                return
            }
            let divided = divided.contains(item)
            // The insets and the minimum height `itemListRow` gives a
            // row in a SwiftUI list.
            cell.contentConfiguration = UIHostingConfiguration {
                AgendaRowView(row: row)
                    .environmentObject(model)
                    .environment(\.openItem) { [weak self] in self?.openItem($0) }
            }
            .margins(.vertical, 4)
            .margins(.horizontal, Self.margin)
            .minSize(height: 44)
            .background {
                if divided {
                    Divider()
                        .padding(.horizontal, Self.margin)
                        .frame(maxHeight: .infinity, alignment: .bottom)
                }
            }
        }
        let header = UICollectionView.SupplementaryRegistration<UICollectionViewListCell>(
            elementKind: UICollectionView.elementKindSectionHeader
        ) { [unowned self] header, _, indexPath in
            setTitle(of: header, section: indexPath.section)
        }
        dataSource = UICollectionViewDiffableDataSource(collectionView: collectionView) { view, indexPath, item in
            view.dequeueConfiguredReusableCell(using: cell, for: indexPath, item: item)
        }
        dataSource.supplementaryViewProvider = { view, _, indexPath in
            view.dequeueConfiguredReusableSupplementary(using: header, for: indexPath)
        }
        if let shown {
            self.shown = nil
            show(shown)
        }
    }

    /// Show these days: Today always, the others when they have rows.
    func show(_ groups: [AgendaGroup]) {
        guard groups != shown else { return }
        shown = groups
        // Before the view has loaded there is nothing to fill;
        // `viewDidLoad` comes back here.
        guard isViewLoaded else { return }

        var snapshot = Snapshot()
        var entries: [Item: AgendaEntry] = [:]
        var titles: [String: String] = [:]
        var anchor: String?
        func append(_ rows: [AgendaEntry], to section: String) {
            for row in rows {
                let item = Item.row(day: section, key: row.key)
                guard entries[item] == nil else { continue }
                entries[item] = row
                snapshot.appendItems([item], toSection: section)
            }
        }
        for day in groups where day.today || !day.rows.isEmpty {
            // A diffable snapshot traps on a repeated identifier.
            guard titles[day.day] == nil else { continue }
            var rows = day.rows
            if day.today {
                anchor = day.day
                let overdue = rows.filter { $0.tone == .overdue }
                rows.removeAll { $0.tone == .overdue }
                if !overdue.isEmpty {
                    titles[Self.overdue] = String(localized: "Overdue")
                    anchor = Self.overdue
                    snapshot.appendSections([Self.overdue])
                    append(overdue, to: Self.overdue)
                }
            }
            titles[day.day] = AgendaView.title(for: day)
            snapshot.appendSections([day.day])
            if rows.isEmpty {
                snapshot.appendItems([.nothingToday], toSection: day.day)
            }
            append(rows, to: day.day)
        }
        var divided: Set<Item> = []
        for section in snapshot.sectionIdentifiers {
            divided.formUnion(snapshot.itemIdentifiers(inSection: section).dropLast())
        }
        // Same row, new content or a line gained or lost: the cell is
        // told to redraw.
        snapshot.reconfigureItems(snapshot.itemIdentifiers.filter { item in
            guard let old = self.entries[item] else { return false }
            return old != entries[item] || self.divided.contains(item) != divided.contains(item)
        })

        let first = self.entries.isEmpty && self.titles.isEmpty
        self.entries = entries
        self.divided = divided
        self.titles = titles
        self.anchor = anchor
        dataSource.apply(snapshot, animatingDifferences: !first && view.window != nil)
        // A header that stayed keeps its view; its words may have
        // changed (midnight moves "Today").
        let kind = UICollectionView.elementKindSectionHeader
        for indexPath in collectionView.indexPathsForVisibleSupplementaryElements(ofKind: kind) {
            if let header = collectionView.supplementaryView(forElementKind: kind, at: indexPath) as? UICollectionViewListCell {
                setTitle(of: header, section: indexPath.section)
            }
        }
        if first, anchor != nil {
            anchoring = true
            view.setNeedsLayout()
        }
    }

    private func setTitle(of header: UICollectionViewListCell, section: Int) {
        let title = dataSource.sectionIdentifier(for: section).flatMap { titles[$0] } ?? ""
        header.backgroundConfiguration = .clear()
        // The Mac's header: the body's size, in the secondary colour.
        header.contentConfiguration = UIHostingConfiguration {
            Text(verbatim: title)
                .foregroundStyle(.secondary)
                .frame(maxWidth: .infinity, alignment: .leading)
                .accessibilityAddTraits(.isHeader)
        }
        .margins(.horizontal, Self.margin)
        .margins(.top, 16)
        .margins(.bottom, 4)
    }

    override func viewDidLayoutSubviews() {
        super.viewDidLayoutSubviews()
        if anchoring { putTodayAtTop() }
    }

    override func viewDidAppear(_ animated: Bool) {
        super.viewDidAppear(animated)
        // The bars have settled: once more, then the scroll is the
        // user's.
        DispatchQueue.main.async { [weak self] in
            guard let self, anchoring else { return }
            putTodayAtTop()
            anchoring = false
        }
    }

    override func scrollViewWillBeginDragging(_ scrollView: UIScrollView) {
        anchoring = false
    }

    /// The "Today" button: scroll back to Today's header (Overdue's,
    /// when it is there), animated.
    func scrollToToday() {
        guard isViewLoaded, let y = todayOffset() else { return }
        anchoring = false
        guard abs(collectionView.contentOffset.y - y) > 0.5 else { return }
        collectionView.setContentOffset(CGPoint(x: collectionView.contentOffset.x, y: y), animated: true)
    }

    /// The scroll to Today has come to rest. Rows measured on the way
    /// can have moved the header from where it was aimed at, so it is
    /// put in place once more.
    override func scrollViewDidEndScrollingAnimation(_ scrollView: UIScrollView) {
        putTodayAtTop()
    }

    /// The offset that has the top of Today's header (Overdue's, when
    /// it is there) at the top of the page, under the bar, so nothing
    /// of the day before shows.
    private func todayOffset() -> CGFloat? {
        guard let anchor, let section = dataSource.snapshot().indexOfSection(anchor),
              let header = collectionView.collectionViewLayout.layoutAttributesForSupplementaryView(
                  ofKind: UICollectionView.elementKindSectionHeader, at: IndexPath(item: 0, section: section)
              )
        else { return nil }
        let inset = collectionView.adjustedContentInset
        // Not past the end: a page shorter than the screen from Today
        // on rests on its last row.
        let end = collectionView.contentSize.height + inset.bottom - collectionView.bounds.height
        return max(-inset.top, min(header.frame.minY - inset.top, end))
    }

    /// Put Today's header at the top of the page. Moving the offset
    /// lays the view out again, which comes back here (while
    /// `anchoring`) until it holds.
    private func putTodayAtTop() {
        guard let y = todayOffset() else { return }
        if abs(collectionView.contentOffset.y - y) > 0.5 {
            collectionView.contentOffset.y = y
        }
    }
}
