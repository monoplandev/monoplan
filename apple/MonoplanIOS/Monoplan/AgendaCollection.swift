// The Calendar page's list, as a UIKit collection view (decided
// 2026-11-17). The page opens on Today with a year of days above it,
// and a SwiftUI `List` cannot be put there reliably: it guesses the
// height of every row it has not built, and `ScrollViewProxy.scrollTo`
// is the only way to move it. A collection view lays out the same
// inset-grouped list, and its layout says where Today's header is, so
// the page sets the offset itself and corrects it on each layout pass
// until it has appeared.
//
// The rows are the SwiftUI ones (`AgendaRowView`), hosted in the cells;
// the headers are the system's list headers.

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
    /// Today's section, the one the page opens on.
    private var today: String?

    /// The page is still to be put on Today: set by the first rows,
    /// cleared once the page has appeared or a finger is on it. While
    /// it is set every layout pass puts Today's header back at the
    /// top, which is what absorbs the bar insets settling during the
    /// push.
    private var anchoring = false

    init(model: AppModel) {
        self.model = model
        var list = UICollectionLayoutListConfiguration(appearance: .insetGrouped)
        list.headerMode = .supplementary
        // No dividers between item rows, as on every page that lists
        // them (`itemListRow`).
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
            guard let row = entries[item] else {
                var content = cell.defaultContentConfiguration()
                content.text = String(localized: "Nothing scheduled")
                content.textProperties.color = .secondaryLabel
                cell.contentConfiguration = content
                return
            }
            // The insets and the minimum height `itemListRow` gives a
            // row in a SwiftUI list.
            cell.contentConfiguration = UIHostingConfiguration {
                AgendaRowView(row: row)
                    .environmentObject(model)
                    .environment(\.openItem) { [weak self] in self?.openItem($0) }
            }
            .margins(.vertical, 4)
            .margins(.horizontal, 20)
            .minSize(height: 44)
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
        var changed: [Item] = []
        var today: String?
        for day in groups where day.today || !day.rows.isEmpty {
            // A diffable snapshot traps on a repeated identifier.
            guard titles[day.day] == nil else { continue }
            titles[day.day] = AgendaView.title(for: day)
            if day.today { today = day.day }
            snapshot.appendSections([day.day])
            if day.rows.isEmpty {
                snapshot.appendItems([.nothingToday], toSection: day.day)
            }
            for row in day.rows {
                let item = Item.row(day: day.day, key: row.key)
                guard entries[item] == nil else { continue }
                entries[item] = row
                snapshot.appendItems([item], toSection: day.day)
                // Same row, new content: the cell is told to redraw.
                if let old = self.entries[item], old != row { changed.append(item) }
            }
        }
        snapshot.reconfigureItems(changed)

        let first = self.entries.isEmpty && self.titles.isEmpty
        self.entries = entries
        self.titles = titles
        self.today = today
        dataSource.apply(snapshot, animatingDifferences: !first && view.window != nil)
        // A header that stayed keeps its view; its words may have
        // changed (midnight moves "Today").
        let kind = UICollectionView.elementKindSectionHeader
        for indexPath in collectionView.indexPathsForVisibleSupplementaryElements(ofKind: kind) {
            if let header = collectionView.supplementaryView(forElementKind: kind, at: indexPath) as? UICollectionViewListCell {
                setTitle(of: header, section: indexPath.section)
            }
        }
        if first, today != nil {
            anchoring = true
            view.setNeedsLayout()
        }
    }

    private func setTitle(of header: UICollectionViewListCell, section: Int) {
        var content = header.defaultContentConfiguration()
        if let day = dataSource.sectionIdentifier(for: section) {
            content.text = titles[day]
        }
        header.contentConfiguration = content
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

    /// The "Today" button: scroll back to Today's header, animated.
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

    /// The offset that has the top of Today's header at the top of the
    /// page, under the bar, so nothing of the day before shows.
    private func todayOffset() -> CGFloat? {
        guard let today, let section = dataSource.snapshot().indexOfSection(today),
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
