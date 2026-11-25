// The one observable the views read: the engine's state (running and
// what it says about sync, or why not yet), the session (account,
// server, open workspaces), every lens and page the views show, and
// the in-flight account work. It owns the `EngineController`; the views
// never touch it.
//
// Everything reaches the engine through the typed calls
// (spec/swift-bindings-plan.md): `call` for a write, `read` for a read,
// both over the running engine's `EngineCalls`, returning the generated
// records (`Item`, `ListInfo`, `AgendaGroup`, ...). The engine's
// `Changed` events arrive through the controller.

import Combine
import Foundation
import MonoplanCore
import MonoplanMacKit
import WidgetKit

/// Who is signed in and which workspaces are open (`Engine.account`,
/// `Engine.workspaces`), read together.
struct Session: Equatable {
    /// Nil for a local-only workspace.
    let account: AccountInfo?
    /// Every open workspace, Home first.
    let docs: [WorkspaceInfo]

    var localOnly: Bool { account == nil }
    var email: String { account?.email ?? "" }
    var serverUrl: String { account?.serverUrl ?? "" }
    var accountId: String { account?.accountId ?? "" }
    var deviceId: String { account?.deviceId ?? "" }
    /// The Personal workspace's doc id.
    var primaryDocId: String { docs.first(where: \.home)?.docId ?? "" }
}

final class AppModel: ObservableObject {
    /// Running with the engine's current sync state, or why not yet.
    @Published private(set) var engineState: EngineState = .connecting(String(localized: "starting"))
    /// Identity, server and open workspaces; nil without an engine.
    @Published private(set) var session: Session?
    /// The profile on disk carries a device token.
    @Published private(set) var hasAccount = false
    /// A sign-in or sign-out in progress.
    @Published private(set) var busy: String?
    /// `Changed` events since launch, and the last one's
    /// time: the cheapest proof that remote ops are arriving.
    @Published private(set) var changeCount = 0
    @Published private(set) var lastChangeAt: Date?
    /// The agenda's day groups (`Engine.agenda`), reread on `Changed`:
    /// in date order, the days gone by ahead of the `today` group.
    @Published private(set) var agenda: [AgendaGroup] = []
    @Published private(set) var agendaError: String?
    /// The Calendar row's count (`Engine.calendarCount`): Open items
    /// owed or on today, as the web's Upcoming badge. Read with the
    /// agenda.
    @Published private(set) var calendarCount = 0
    /// The Calendar filter's checked workspaces (`Engine.calendarCheckedDocs`,
    /// read off Home's synced hidden set, which the web's and the Mac's
    /// filter write too), read with the agenda; none checked is every
    /// workspace.
    @Published private(set) var calendarCheckedDocs: [String] = []
    /// The Starred lens (`Engine.starred`): Home's curated order with
    /// its cross-workspace entries resolved.
    @Published private(set) var starred: [StarredItem] = []
    @Published private(set) var starredError: String?
    /// The Assigned lens (`Engine.assigned`): the open items assigned
    /// to this account in every open workspace, reread on `Changed`.
    @Published private(set) var assigned: [AssignedEntry] = []
    @Published private(set) var assignedError: String?
    /// The combined Inbox (`Engine.inbox`, spec/inbox-plan.md "One
    /// Inbox"): every open workspace's Inbox as one queue, oldest
    /// first, reread on `Changed`. Home's Inbox row counts it.
    @Published private(set) var inbox: [InboxEntry] = []
    @Published private(set) var inboxError: String?
    /// The Open items that carry a place, every workspace, and the
    /// closed ones too when the Map page's filter asks for them
    /// (`Engine.placedItems`): what the Map page shows. Read while that
    /// page is up.
    @Published private(set) var placed: [LocatedItem] = []
    @Published private(set) var placedError: String?
    /// Whether the account owns each shared workspace, by doc id, from
    /// its member list (`Engine.workspaceMembers`): Home's "..." menu
    /// offers Delete to an owner and Leave to anyone else, and neither
    /// while this is unknown (offline, or still loading), as the web's
    /// menu waits for its member directory.
    @Published private(set) var ownsWorkspace: [String: Bool] = [:]
    /// How many members each shared workspace has, by doc id, from the
    /// same member list: Delete List's confirm says who will see the
    /// items it sends to the Inbox.
    @Published private(set) var memberCounts: [String: Int] = [:]
    /// Each shared workspace's members, by doc id, from the same member
    /// list: who an item there can be assigned to, and the email an
    /// assignee's account id shows as (spec/sharing-plan.md
    /// "Assignment"). Absent while the list is not known.
    @Published private(set) var members: [String: [WorkspaceMember]] = [:]
    /// Shared workspaces whose role is being asked for now.
    private var rolesLoading: Set<String> = []
    /// The Done lens (`Engine.doneEverywhere`): every open workspace's
    /// newest `doneLimit` Done items that `doneFilter` admits, grouped
    /// by the day each was ticked. A window, never the whole history: the view
    /// grows it as the end scrolls into sight (`loadMoreDone`), and a
    /// `Changed` refetches only as far as has been scrolled.
    @Published private(set) var doneDays: [DoneDay] = []
    @Published private(set) var doneError: String?
    /// The window `doneDays` was fetched with, and how many rows came
    /// back: a full window means there may be more.
    @Published private(set) var doneLoadedLimit = 0
    private var doneCount = 0
    /// How many Done items the filter admits in all
    /// (`Engine.doneCountEverywhere`), loaded or not; nil until first read.
    @Published private(set) var doneTotal: Int?
    /// The Done page's filter as chosen (`DoneFilterView.swift`), kept
    /// verbatim; what is applied and shown is `doneFilter`.
    @Published private(set) var doneFilterChoice = DoneFilterStore.load()
    /// The filter the rows on screen were fetched with.
    private var doneFetchedFilter: DoneFilter?
    private var doneLimit = AppModel.donePage
    static let donePage = 50
    /// The bin (`Engine.binned`): every open workspace's, newest binned
    /// first. Read with the other lenses: Home shows its Bin row only
    /// while there is something in it.
    @Published private(set) var binned: [BinEntry] = []
    @Published private(set) var binError: String?
    /// The bin has been read at least once.
    @Published private(set) var binLoaded = false
    /// Each open workspace's lists, archived ones included
    /// (`Engine.lists` per doc), keyed by doc id: the Home index.
    /// Refetched with the session.
    @Published private(set) var listsByDoc: [String: [ListInfo]] = [:]
    /// A browsed list's Open items (`Engine.items`), keyed by
    /// `ListRef.key`, fetched when its page appears and again on each
    /// `Changed` while it is shown.
    @Published private(set) var listItems: [String: [Item]] = [:]
    @Published private(set) var listErrors: [String: String] = [:]
    /// The lists opened so far, to reread after a write of ours.
    private var browsedLists: [String: ListRef] = [:]
    /// An opened item (`Engine.item`), keyed by `ItemRef.key`, read
    /// when its page appears and on each `Changed` while shown.
    @Published private(set) var itemDetails: [String: ItemInfo] = [:]
    @Published private(set) var itemErrors: [String: String] = [:]
    /// Search results (`Engine.find`: every open workspace, ranked),
    /// and the query they answer. The hits of the query before
    /// stay until the next reply, so `searchedQuery` says whether they
    /// are for what is typed now.
    @Published private(set) var searchHits: [FindHit] = []
    @Published private(set) var searchedQuery = ""
    @Published private(set) var searchError: String?
    /// What is being searched for, trimmed; empty while search is idle.
    private var searchQuery = ""
    static let searchLimit: UInt32 = 50
    private var refreshQueued = false
    @Published var lastError: String?
    @Published var signingIn = false
    @Published var signInError: String?
    @Published var deletingAccount = false
    @Published var deleteAccountError: String?

    /// The item whose reminder was tapped, for the item sheet to open
    /// (`opensItems()`), which then calls `showedReminder`. Held back
    /// until the session has loaded: a tap can be what starts the app.
    @Published private(set) var tappedReminder: ItemRef?
    private var awaitedReminder: ItemRef?
    private let reminders = ReminderScheduler.shared
    /// A place alert for several tasks was tapped, for Home to show
    /// the Map page, which then calls `showedPlaces`.
    @Published private(set) var tappedPlaces = false
    /// The phone's location, as far as the place alerts need it
    /// (spec/place-alerts.md); idle while they are off.
    let placeLocation = PlaceAlertLocation()

    let location: ProfileLocation
    private let engineController: EngineController

    init() {
        location = ProfileLocation.resolve()
        // Application Support does not exist in a fresh iOS container;
        // the Rust side creates the profile dir itself but not its
        // parents.
        try? FileManager.default.createDirectory(at: location.dir, withIntermediateDirectories: true)
        engineController = EngineController(location: location)
        hasAccount = location.hasAccount
        engineController.onState = { [weak self] state in self?.stateChanged(state) }
        engineController.onChanged = { [weak self] in self?.docChanged() }
        reminders.leadText = { String(localized: "In \($0) minutes") }
        reminders.onOpen = { [weak self] tap in
            self?.openReminder(ItemRef(docId: tap.docId, itemId: tap.itemId, title: tap.title))
        }
        reminders.moreText = { String(localized: "and \($0) more") }
        reminders.onOpenPlaces = { [weak self] in self?.tappedPlaces = true }
        placeLocation.onChange = { [weak self] in self?.refreshPlaceAlerts() }
        if PlaceAlertSettings.isOn() { placeLocation.locate() }
        engineController.start()
    }

    /// The sync state while the engine runs.
    var sync: SyncState? {
        if case .connected(let sync) = engineState { return sync }
        return nil
    }

    /// A new engine, or the running one's sync state moved (it went
    /// online, a pull landed): either way what is open may have changed.
    private func stateChanged(_ state: EngineState) {
        engineState = state
        hasAccount = location.hasAccount
        switch state {
        case .connected:
            loadSession()
            refreshAgenda()
            refreshStarred()
            refreshAssigned()
            refreshInbox()
            refreshDone()
            refreshReminders()
        case .connecting:
            session = nil
        }
    }

    private func docChanged() {
        changeCount += 1
        lastChangeAt = Date()
        scheduleRefresh()
    }

    /// Coalesce a burst of `Changed` events into one round of fetches,
    /// as the Mac tray does for its agenda.
    private func scheduleRefresh() {
        if refreshQueued { return }
        refreshQueued = true
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.25) { [weak self] in
            self?.refreshQueued = false
            self?.refreshLenses()
            // Workspaces come and go (created, shared, closed) and lists
            // are renamed: the index follows the same events.
            self?.loadSession()
        }
    }

    private func refreshLenses() {
        refreshAgenda()
        refreshStarred()
        refreshAssigned()
        refreshInbox()
        refreshDone()
        refreshSearch()
        refreshBin()
        refreshReminders()
    }

    /// Search for what the bar's field holds, on every keystroke: the
    /// index is in process and a query is well under a frame. An empty
    /// query clears the results; it is not a request.
    func search(_ query: String) {
        let query = query.trimmingCharacters(in: .whitespacesAndNewlines)
        guard query != searchQuery else { return }
        searchQuery = query
        if query.isEmpty {
            searchHits = []
            searchedQuery = ""
            searchError = nil
        } else {
            refreshSearch()
        }
    }

    /// Run the current query again: it changed, or the docs did under
    /// it. A reply for a query no longer typed is dropped.
    private func refreshSearch() {
        let query = searchQuery
        guard !query.isEmpty else { return }
        read({ try $0.find(query: query, limit: Self.searchLimit) }) { [weak self] hits, error in
            guard let self, query == searchQuery else { return }
            if let hits {
                searchHits = hits
                searchedQuery = query
            }
            searchError = error
        }
    }

    /// The item a capture last created and the list it landed in (or
    /// the item an item page's "Show in List" asked for, `reveal`),
    /// until that list's page has shown it: Home goes to the list, and
    /// the list scrolls to the item and takes this back
    /// (`showedCaptured`).
    @Published private(set) var captured: Captured?

    struct Captured: Equatable {
        let list: ListRef
        let itemId: String
    }

    /// The list page has the captured item on screen.
    func showedCaptured() { captured = nil }

    /// Go to `list` and scroll to the item there, as after a capture.
    func reveal(_ itemId: String, in list: ListRef) {
        captured = Captured(list: list, itemId: itemId)
    }

    /// Capture: create an item at the top of `list` (an Inbox's at
    /// the end of its queue, whatever is asked) (`Engine.addItem`),
    /// then reread what is on screen as `setDone` does. The engine
    /// creates it in Backlog; any other `state` is a second write
    /// straight after. `done` gets the new item, or why there is none;
    /// it may be left out by a caller that is going away (a capture
    /// sheet closing over a typed title).
    func addItem(
        text: String, to list: ListRef, state: ItemState = .backlog, whenSuggested: String? = nil,
        done: ((ItemRef?, String?) -> Void)? = nil
    ) {
        let workspace = selector(list.docId)
        let listId = list.listId
        call({ try $0.addItem(workspace: workspace, list: listId, text: text, top: true) }) { [weak self] result in
            guard let self else { return }
            switch result {
            case .success(let id):
                refreshLenses()
                refreshListRows()
                for list in browsedLists.values { refreshList(list) }
                let ref = ItemRef(docId: list.docId, itemId: id, title: text)
                captured = Captured(list: list, itemId: id)
                // The date its title named, as a question on the
                // new item (`WhenSuggest`).
                if let whenSuggested { setWhenSuggested(ref, when: whenSuggested) }
                if state != .backlog { setState(ref, state: state) }
                done?(ref, nil)
            case .failure(let error):
                let failure = String(localized: "could not add item: \(engineErrorMessage(error))")
                lastError = failure
                done?(nil, failure)
            }
        }
    }

    /// Tick an item, or reopen a closed one (`Engine.setDone`).
    /// Nothing is assumed about the result: once the engine has applied
    /// it, everything on screen is read again at once rather than after
    /// the `Changed` coalescing delay, so the tick lands in the same
    /// frame or two. In process, the write and the reads are
    /// milliseconds.
    func setDone(_ ref: ItemRef, done: Bool) {
        let workspace = selector(ref.docId)
        let id = ref.itemId
        if done { DoneSound.play() }
        call({ try $0.setDone(workspace: workspace, id: id, done: done) }) { [weak self] result in
            switch result {
            case .success: self?.wrote(to: ref)
            case .failure(let error):
                self?.lastError = String(localized: "could not update item: \(engineErrorMessage(error))")
            }
        }
    }

    /// Move an item to any workflow state (`Engine.setState`), then
    /// reread as `setDone` does.
    func setState(_ ref: ItemRef, state: ItemState) {
        let (workspace, id) = (selector(ref.docId), ref.itemId)
        if state == .done { DoneSound.play() }
        write({ try $0.setState(workspace: workspace, id: id, state: state) }, to: ref)
    }

    /// Retitle an item (`Engine.setText`), with what the new title does
    /// to its date suggestion, then reread as `setDone` does.
    func setText(_ ref: ItemRef, text: String, suggestion: SuggestionChange = .keep) {
        let (workspace, id) = (selector(ref.docId), ref.itemId)
        write({ try $0.setText(workspace: workspace, id: id, text: text, suggestion: suggestion) }, to: ref)
    }

    /// Raise a date suggestion on an item, or dismiss the pending one
    /// (nil: the user's "no"). The "yes" is `setWhen` with the
    /// suggested value: any date set clears the suggestion.
    func setWhenSuggested(_ ref: ItemRef, when: String?) {
        let (workspace, id) = (selector(ref.docId), ref.itemId)
        write({ try $0.setWhenSuggested(workspace: workspace, id: id, when: when) }, to: ref)
    }

    /// Move an item to another list of its own workspace
    /// (`Engine.moveItem`), then reread as `setDone` does.
    func moveItem(_ ref: ItemRef, toList listId: String) {
        let (workspace, id) = (selector(ref.docId), ref.itemId)
        write({ try $0.moveItem(workspace: workspace, id: id, list: listId) }, to: ref)
    }

    /// Move an item to a list of another workspace
    /// (`Engine.moveItemAcross`, spec/sharing-plan.md "Cross-doc
    /// move"): it keeps its id and its star, and lands at the end of
    /// the list. `done` gets the item where it now lives, its page
    /// carried over until the reread lands. The caller has confirmed
    /// when the audience changes (`moveChangesAudience`).
    func moveItem(_ ref: ItemRef, across list: ListRef, done: @escaping (ItemRef) -> Void) {
        let (source, target, id) = (selector(ref.docId), selector(list.docId), ref.itemId)
        let moved = ItemRef(docId: list.docId, itemId: id, title: ref.title)
        call({
            try $0.moveItemAcross(
                item: MonoplanCore.ItemRef(workspace: source, id: id), to: target, list: list.listId)
        }) { [weak self] result in
            guard let self else { return }
            switch result {
            case .success:
                if let page = itemDetails[ref.key] {
                    itemDetails[moved.key] = page
                    itemDetails[ref.key] = nil
                }
                done(moved)
                wrote(to: moved)
            case .failure(let error):
                lastError = String(localized: "could not move item: \(engineErrorMessage(error))")
            }
        }
    }

    /// What the user is told before an item of `source` moves to a
    /// list of `target`; nil when the move changes nobody's view of
    /// it (between the account's own unshared workspaces) and so just
    /// happens. As the web: into a workspace with other members it
    /// says who will see the item, out of one that it goes for
    /// everyone there.
    func moveChangesAudience(from source: String?, to target: String) -> String? {
        var lines: [String] = []
        if hasOtherMembers(target) {
            let name = workspaceLabel(target)
            lines.append(
                memberCounts[target].map { String(localized: "This item will be visible to all \($0) members of \(name).") }
                    ?? String(localized: "This item will be visible to everyone in \(name)."))
        }
        if let source, hasOtherMembers(source) {
            lines.append(String(localized: "It will be removed from \(workspaceLabel(source)) for everyone there."))
        }
        return lines.isEmpty ? nil : lines.joined(separator: " ")
    }

    /// A workspace's name as Home heads it.
    func workspaceLabel(_ docId: String?) -> String {
        sharedWorkspaceName(docId) ?? String(localized: "Personal")
    }

    /// Every other open workspace with the lists an item of `docId`
    /// (nil: Personal) can be moved to there, Home first.
    func otherLists(thanDoc docId: String?) -> [ListGroup] {
        guard let session else { return [] }
        let own = docId ?? session.primaryDocId
        return session.docs.filter { $0.docId != own }.map {
            ListGroup(
                docId: $0.docId, name: workspaceLabel($0.docId), shared: !$0.home,
                lists: lists(inDoc: $0.docId))
        }
    }

    /// A row of a browsed list dragged to a new place, in `onMove`'s
    /// terms: `destination` is the gap it was dropped in, counted in
    /// the order as it was. The rows are reordered here first, so the
    /// row stays where it was dropped while the write and its reread
    /// are on their way; the engine takes the 1-based slot the item
    /// ends up in (`Engine.reorderItem`). The list is reread either way: a
    /// refused move puts the row back.
    func reorderItem(in list: ListRef, from source: IndexSet, to destination: Int) {
        guard var items = listItems[list.key], source.count == 1, let from = source.first,
              items.indices.contains(from)
        else { return }
        let slot = destination > from ? destination - 1 : destination
        guard slot != from else { return }
        let id = items[from].id
        items.move(fromOffsets: source, toOffset: destination)
        listItems[list.key] = items
        let (workspace, pos) = (selector(list.docId), UInt32(slot + 1))
        call({ try $0.reorderItem(workspace: workspace, id: id, pos: pos) }) { [weak self] result in
            guard let self else { return }
            if case .failure(let error) = result {
                lastError = String(localized: "could not move item: \(engineErrorMessage(error))")
            }
            refreshList(list)
        }
    }

    /// A Starred row dragged to a new place, in `onMove`'s terms, as
    /// `reorderItem` has a list's: reordered here first, then written
    /// and reread. The engine is told the row it now sits under
    /// (`Engine.moveStarred`), or the top: Starred's positions count
    /// entries this page does not show.
    func moveStarred(from source: IndexSet, to destination: Int) {
        var rows = starred
        guard source.count == 1, let from = source.first, rows.indices.contains(from) else { return }
        let slot = destination > from ? destination - 1 : destination
        guard slot != from else { return }
        let id = rows[from].id
        rows.move(fromOffsets: source, toOffset: destination)
        starred = rows
        let after = slot > 0 ? rows[slot - 1].id : nil
        call({ try $0.moveStarred(id: id, after: after) }) { [weak self] result in
            guard let self else { return }
            if case .failure(let error) = result {
                lastError = String(localized: "could not move item: \(engineErrorMessage(error))")
            }
            refreshStarred()
        }
    }

    /// Star an item (to the top of Starred) or unstar it
    /// (`Engine.setStarred`). Starred is the Home doc's whichever
    /// workspace the item is in; the engine sees to that.
    func setStarred(_ ref: ItemRef, starred: Bool) {
        let (workspace, id) = (selector(ref.docId), ref.itemId)
        write({ try $0.setStarred(workspace: workspace, id: id, starred: starred) }, to: ref)
    }

    /// Move an item to the bin, or restore it from there
    /// (`Engine.setBinned`), then reread as `setDone` does.
    func setBinned(_ ref: ItemRef, binned: Bool) {
        let (workspace, id) = (selector(ref.docId), ref.itemId)
        write({ try $0.setBinned(workspace: workspace, id: id, binned: binned) }, to: ref)
    }

    /// Move a list page's selected rows, in the order given, to the end
    /// of another list of their workspace (`Engine.moveItems`): one
    /// call, so one undo step.
    func moveItems(_ ids: [String], inDoc docId: String?, toList listId: String) {
        let workspace = selector(docId)
        // No position is past this: the core clamps to the list's end.
        writeMany { try $0.moveItems(workspace: workspace, ids: ids, list: listId, index: .max) }
    }

    /// Move a selection to the bin (`Engine.setBinnedMany`), as one
    /// undo step.
    func binItems(_ refs: [ItemRef]) {
        let items = refs.map { MonoplanCore.ItemRef(workspace: selector($0.docId), id: $0.itemId) }
        writeMany { try $0.setBinnedMany(items: items, binned: true) }
    }

    /// Star a selection, on top of Starred in the order given
    /// (`Engine.starItems`); rows already starred stay where they are.
    func starItems(_ refs: [ItemRef]) {
        let items = refs.map { MonoplanCore.ItemRef(workspace: selector($0.docId), id: $0.itemId) }
        writeMany { try $0.starItems(items: items) }
    }

    /// Unstar a selection. There is no call for many, so each is
    /// unstarred in turn (`Engine.setStarred`) inside the one call.
    func unstarItems(_ refs: [ItemRef]) {
        let items = refs.map { (selector($0.docId), $0.itemId) }
        writeMany {
            for (workspace, id) in items {
                try $0.setStarred(workspace: workspace, id: id, starred: false)
            }
        }
    }

    /// A write to several items, then everything that lists items
    /// reread.
    private func writeMany(_ body: @escaping (MonoplanCore.Engine) throws -> Void) {
        call(body) { [weak self] result in
            guard let self else { return }
            switch result {
            case .success:
                refreshLenses()
                refreshListRows()
                for list in browsedLists.values { refreshList(list) }
            case .failure(let error):
                lastError = String(localized: "could not update items: \(engineErrorMessage(error))")
            }
        }
    }

    /// Copy an item beside itself in its own list
    /// (`Engine.duplicateItem`), then reread as `setDone` does. The
    /// copy's id is not needed: the reread shows it. `star` also stars
    /// the copy of a starred item, directly below it in Starred.
    func duplicateItem(_ ref: ItemRef, star: Bool = false) {
        let (workspace, id) = (selector(ref.docId), ref.itemId)
        write({ _ = try $0.duplicateItem(workspace: workspace, id: id, star: star) }, to: ref)
    }

    /// Hard-delete a binned item (`Engine.deleteBinned`). Its page is
    /// forgotten first, so nothing rereads an item that is no longer
    /// there.
    func deleteBinned(_ ref: ItemRef) {
        itemDetails[ref.key] = nil
        let (workspace, id) = (selector(ref.docId), ref.itemId)
        write({ try $0.deleteBinned(workspace: workspace, id: id) }, to: ref)
    }

    /// Hard-delete everything in the bin, a workspace at a time
    /// (`Engine.emptyBin`), then reread as `setDone` does. Only the
    /// workspaces with something binned are asked.
    func emptyBin() {
        for item in binned { itemDetails[item.ref.key] = nil }
        for docId in Set(binned.map(\.docId)) {
            let workspace = selector(docId)
            call({ try $0.emptyBin(workspace: workspace) }) { [weak self] result in
                guard let self else { return }
                switch result {
                case .success: refreshLenses()
                case .failure(let error):
                    lastError = String(localized: "could not empty the bin: \(engineErrorMessage(error))")
                }
            }
        }
    }

    /// `Engine.binned`: every open workspace's bin, newest binned first.
    func refreshBin() {
        read({ try $0.binned() }) { [weak self] rows, error in
            guard let self else { return }
            if let rows { binned = rows }
            binError = error
            binLoaded = true
        }
    }

    /// The name of a list in any open workspace, for a row that names
    /// the list its item is in. `dated` is whether that item has a
    /// date: with no list it then reads as the Calendar's.
    func listName(_ listId: String, inDoc docId: String?, dated: Bool = false) -> String? {
        if listId == "inbox" { return NoList.label(dated: dated) }
        return (listsByDoc[docId ?? session?.primaryDocId ?? ""] ?? []).names[listId]
    }

    /// Set or clear (nil) an item's planned date, with what to do to
    /// its duration in the same call (`Engine.setWhen`).
    func setWhen(_ ref: ItemRef, when: String?, duration: DurationChange = .keep) {
        let (workspace, id) = (selector(ref.docId), ref.itemId)
        write({ try $0.setWhen(workspace: workspace, id: id, when: when, duration: duration) }, to: ref)
    }

    /// Fix an item's timed date to an IANA zone, its clock digits
    /// kept, or make it floating again at the time this device shows
    /// (nil) (`Engine.setWhenZone`).
    func setWhenZone(_ ref: ItemRef, zone: String?) {
        let (workspace, id) = (selector(ref.docId), ref.itemId)
        write({ try $0.setWhenZone(workspace: workspace, id: id, zone: zone) }, to: ref)
    }

    /// Mark a dated item an event, or a task again (`Engine.setEvent`).
    func setEvent(_ ref: ItemRef, event: Bool) {
        let (workspace, id) = (selector(ref.docId), ref.itemId)
        write({ try $0.setEvent(workspace: workspace, id: id, event: event) }, to: ref)
    }

    /// Assign an item to one member of its workspace by account id, or
    /// unassign it (nil) (`Engine.setAssignee`).
    func setAssignee(_ ref: ItemRef, assignee: String?) {
        let (workspace, id) = (selector(ref.docId), ref.itemId)
        write({ try $0.setAssignee(workspace: workspace, id: id, assignee: assignee) }, to: ref)
    }

    /// The item page's subtask writes (spec/subtasks.md), one typed
    /// call and one undo step each: add at the end, retitle, tick or
    /// untick, move to an index, remove.
    func addSubtask(_ ref: ItemRef, text: String) {
        let (workspace, id) = (selector(ref.docId), ref.itemId)
        write({ _ = try $0.addSubtask(workspace: workspace, id: id, text: text, index: nil) }, to: ref)
    }

    func setSubtaskText(_ ref: ItemRef, subtask: String, text: String) {
        let (workspace, id) = (selector(ref.docId), ref.itemId)
        write({ try $0.setSubtaskText(workspace: workspace, id: id, subtask: subtask, text: text) }, to: ref)
    }

    func setSubtaskDone(_ ref: ItemRef, subtask: String, done: Bool) {
        let (workspace, id) = (selector(ref.docId), ref.itemId)
        write({ try $0.setSubtaskDone(workspace: workspace, id: id, subtask: subtask, done: done) }, to: ref)
    }

    func moveSubtask(_ ref: ItemRef, subtask: String, index: Int) {
        let (workspace, id) = (selector(ref.docId), ref.itemId)
        let index = UInt32(clamping: index)
        write({ try $0.moveSubtask(workspace: workspace, id: id, subtask: subtask, index: index) }, to: ref)
    }

    func removeSubtask(_ ref: ItemRef, subtask: String) {
        let (workspace, id) = (selector(ref.docId), ref.itemId)
        write({ try $0.removeSubtask(workspace: workspace, id: id, subtask: subtask) }, to: ref)
    }

    /// Set or clear (nil) an item's duration in minutes.
    func setDuration(_ ref: ItemRef, minutes: Int?) {
        let (workspace, id) = (selector(ref.docId), ref.itemId)
        let minutes = minutes.map { UInt32(clamping: $0) }
        write({ try $0.setDuration(workspace: workspace, id: id, minutes: minutes) }, to: ref)
    }

    /// Set an item's place, or clear it (nil). `exact` writes a bare
    /// label as just that, instead of letting the engine take a known
    /// place of the same label (`Engine.setPlace`).
    func setPlace(_ ref: ItemRef, place: Place?, exact: Bool = false) {
        let (workspace, id) = (selector(ref.docId), ref.itemId)
        write({ try $0.setPlace(workspace: workspace, id: id, place: place, exact: exact) }, to: ref)
    }

    /// The account's known places matching `query` (all when empty),
    /// most recently used first: what the place editor offers before
    /// any lookup leaves the device. Nothing comes back on failure.
    func knownPlaces(matching query: String, limit: UInt32 = 8, completion: @escaping ([PlaceSuggestion]) -> Void) {
        read({ try $0.places(query: query, limit: limit) }) { rows, _ in
            if let rows { completion(rows) }
        }
    }

    /// The lists an item of this workspace can be in: its Inbox, then
    /// its active lists in the doc's order.
    func lists(inDoc docId: String?) -> [ListRef] {
        guard let docId = docId ?? session?.primaryDocId else { return [] }
        return [ListRef(docId: docId, listId: "inbox", name: String(localized: "Inbox"))]
            + (listsByDoc[docId] ?? []).userLists.map {
                ListRef(docId: docId, listId: $0.id, name: $0.name, icon: $0.icon)
            }
    }

    /// Create a list at the end of a workspace's lists
    /// (`Engine.addList`), then reread Home's list rows. `done` gets
    /// the new list, or why there is none.
    func addList(name: String, inDoc docId: String, done: @escaping (ListRef?, String?) -> Void) {
        let name = name.trimmingCharacters(in: .whitespacesAndNewlines)
        let workspace = selector(docId)
        call({ try $0.addList(workspace: workspace, name: name) }) { [weak self] result in
            guard let self else { return }
            switch result {
            case .success(let id):
                refreshListRows()
                done(ListRef(docId: docId, listId: id, name: name), nil)
            case .failure(let error):
                let failure = String(localized: "could not add list: \(engineErrorMessage(error))")
                lastError = failure
                done(nil, failure)
            }
        }
    }

    /// Create a workspace the account owns, by name
    /// (`Engine.createWorkspace`): the engine opens it, and the
    /// session reread puts its section on Home. `done` gets why not,
    /// or nil once it exists.
    func createWorkspace(name: String, done: @escaping (String?) -> Void) {
        call({ try $0.createWorkspace(name: name) }) { [weak self] result in
            switch result {
            case .success:
                self?.loadSession()
                done(nil)
            case .failure(let error):
                done(engineErrorMessage(error))
            }
        }
    }

    /// Rename a shared workspace, for every member
    /// (`Engine.renameWorkspace`). `done` gets why not, or nil once it
    /// is renamed; the session reread retitles its section.
    func renameWorkspace(_ docId: String, to name: String, done: @escaping (String?) -> Void) {
        call({ try $0.renameWorkspace(workspace: docId, name: name) }) { [weak self] result in
            switch result {
            case .success:
                self?.loadSession()
                self?.refreshLenses()
                done(nil)
            case .failure(let error):
                done(engineErrorMessage(error))
            }
        }
    }

    /// Delete a workspace the account owns, for every device
    /// (`Engine.deleteWorkspace`). The server refuses one that is not
    /// ours or still has other members; `done` gets why not, or nil
    /// once it is gone.
    func deleteWorkspace(_ docId: String, done: @escaping (String?) -> Void) {
        call({ try $0.deleteWorkspace(docId: docId) }) { [weak self] result in
            switch result {
            case .success:
                self?.loadSession()
                self?.refreshLenses()
                done(nil)
            case .failure(let error):
                done(engineErrorMessage(error))
            }
        }
    }

    /// Leave a shared workspace the account does not own
    /// (`Engine.leaveWorkspace`). The engine forgets it once the
    /// server's `DocClosed` lands, and the `Changed` that follows
    /// rereads the session; the reread here only makes it sooner.
    /// `done` gets why not, or nil once the server has let go.
    func leaveWorkspace(_ docId: String, done: @escaping (String?) -> Void) {
        sharing({ try $0.leaveWorkspace(docId: docId) }) { [weak self] result in
            switch result {
            case .success:
                self?.ownsWorkspace[docId] = nil
                self?.loadSession()
                self?.refreshLenses()
                done(nil)
            case .failure(let error):
                done(engineErrorMessage(error))
            }
        }
    }

    /// One of the engine's sharing calls (`Engine.workspaceMembers`,
    /// `createInvite`, ...). Each is a round trip to the server, so it
    /// runs on a queue of its own rather than the doc calls' serial
    /// one: an unreachable server must not hold up a tick. `done` gets
    /// the result on the main queue.
    func sharing<T>(
        _ body: @escaping (MonoplanCore.Engine) throws -> T,
        done: @escaping (Result<T, Error>) -> Void
    ) {
        guard let engine = engineController.calls?.engine else {
            DispatchQueue.main.async { done(.failure(EngineError.Stopped)) }
            return
        }
        DispatchQueue.global(qos: .userInitiated).async {
            let result = Result { try body(engine) }
            DispatchQueue.main.async { done(result) }
        }
    }

    /// What a member list says of the account's role on a workspace:
    /// the Sharing sheet reports each list it loads, so the menu
    /// follows a change it shows.
    func noteMembers(_ members: [WorkspaceMember], of docId: String) {
        guard let me = session?.accountId, !me.isEmpty else { return }
        ownsWorkspace[docId] = members.first(where: { $0.accountId == me })?.owner ?? false
        memberCounts[docId] = members.count
        self.members[docId] = members.sorted { $0.email < $1.email }
    }

    /// Whether a workspace has members besides the account: false for
    /// the Personal workspace; a shared one whose member list is not
    /// known yet (offline, still loading) counts as having some, so
    /// Delete List's note errs on saying who will see.
    func hasOtherMembers(_ docId: String) -> Bool {
        guard sharedWorkspaceName(docId) != nil else { return false }
        return (memberCounts[docId] ?? 2) > 1
    }

    /// Ask for the role on every shared workspace not yet known, and
    /// forget the ones that have closed. Once per workspace per run of
    /// the app, not per session reread: the reread follows every burst
    /// of changes, and roles only change by transfer, which is not
    /// built.
    private func refreshRoles() {
        guard let session, !session.localOnly else {
            ownsWorkspace = [:]
            memberCounts = [:]
            members = [:]
            return
        }
        let shared = Set(session.docs.filter { !$0.home }.map(\.docId))
        for gone in ownsWorkspace.keys where !shared.contains(gone) { ownsWorkspace[gone] = nil }
        for gone in memberCounts.keys where !shared.contains(gone) { memberCounts[gone] = nil }
        for gone in members.keys where !shared.contains(gone) { members[gone] = nil }
        for docId in shared where ownsWorkspace[docId] == nil && !rolesLoading.contains(docId) {
            rolesLoading.insert(docId)
            sharing({ try $0.workspaceMembers(docId: docId) }) { [weak self] result in
                guard let self else { return }
                rolesLoading.remove(docId)
                // A failure leaves it unknown; the next reread asks again.
                if case .success(let members) = result { noteMembers(members, of: docId) }
            }
        }
    }

    /// Rename a user-created list (`Engine.renameList`).
    func renameList(_ list: ListRef, to name: String) {
        let (workspace, id) = (selector(list.docId), list.listId)
        writeList({ try $0.renameList(workspace: workspace, list: id, name: name) })
    }

    /// Set a list's emoji, or clear it (nil) (`Engine.setListIcon`).
    func setListIcon(_ list: ListRef, icon: String?) {
        let (workspace, id) = (selector(list.docId), list.listId)
        writeList({ try $0.setListIcon(workspace: workspace, list: id, icon: icon) })
    }

    /// Archive a list (`Engine.setListArchived`): it leaves Home, its
    /// items kept. `done` gets why not, or nil once it is archived.
    func archiveList(_ list: ListRef, done: @escaping (String?) -> Void) {
        let (workspace, id) = (selector(list.docId), list.listId)
        writeList({ try $0.setListArchived(workspace: workspace, list: id, archived: true) }, done: done)
    }

    /// Delete a user-created list (`Engine.deleteList`): its open items
    /// go to the Bin, its done and cancelled ones to the Inbox, closed. One undo step restores the list and its items.
    /// `done` gets why not, or nil once it is gone; the lists on screen
    /// are reread.
    func deleteList(_ list: ListRef, done: @escaping (String?) -> Void) {
        let (workspace, id) = (selector(list.docId), list.listId)
        writeList({ try $0.deleteList(workspace: workspace, list: id, mode: .toBin) }) { [weak self] error in
            if error == nil, let self {
                browsedLists[list.key] = nil
                for list in browsedLists.values { refreshList(list) }
            }
            done(error)
        }
    }

    /// A list edit, then Home's list rows reread; item rows elsewhere
    /// name their list, so the lenses are reread too.
    private func writeList(
        _ body: @escaping (MonoplanCore.Engine) throws -> Void, done: ((String?) -> Void)? = nil
    ) {
        call(body) { [weak self] result in
            guard let self else { return }
            switch result {
            case .success:
                refreshListRows()
                refreshLenses()
                done?(nil)
            case .failure(let error):
                let failure = String(localized: "could not update list: \(engineErrorMessage(error))")
                lastError = failure
                done?(failure)
            }
        }
    }

    /// An item write, then everything that may show the item reread.
    private func write(_ body: @escaping (MonoplanCore.Engine) throws -> Void, to ref: ItemRef) {
        call(body) { [weak self] result in
            switch result {
            case .success: self?.wrote(to: ref)
            case .failure(let error):
                self?.lastError = String(localized: "could not update item: \(engineErrorMessage(error))")
            }
        }
    }

    /// An item write of ours has been applied: read again everything
    /// that may show it.
    private func wrote(to ref: ItemRef) {
        refreshLenses()
        if itemDetails[ref.key] != nil { refreshItem(ref) }
        refreshListRows()
        for list in browsedLists.values { refreshList(list) }
    }

    /// One typed call on the running engine (`EngineCalls`: off the
    /// main thread, in the order made); `done` gets the result back on
    /// the main queue. With no engine it fails as a stopped one does.
    private func call<T>(
        _ body: @escaping (MonoplanCore.Engine) throws -> T,
        done: @escaping (Result<T, Error>) -> Void
    ) {
        guard let calls = engineController.calls else {
            DispatchQueue.main.async { done(.failure(EngineError.Stopped)) }
            return
        }
        calls.run(body, completion: done)
    }

    /// A typed read. No engine is not an error: the rows stay until
    /// the next engine's refresh replaces them, so `done` gets neither
    /// rows nor an error then.
    private func read<T>(
        _ body: @escaping (MonoplanCore.Engine) throws -> T,
        done: @escaping (T?, String?) -> Void
    ) {
        call(body) { result in
            switch result {
            case .success(let value): done(value, nil)
            case .failure(EngineError.Stopped): return
            case .failure(let error): done(nil, engineErrorMessage(error))
            }
        }
    }

    /// A shared workspace's name: nil for the Personal workspace (and
    /// for a doc that is not open), as the web's `isSharedDoc` has any
    /// doc but Home shared.
    func sharedWorkspaceName(_ docId: String?) -> String? {
        guard let session, let docId, docId != session.primaryDocId,
              let doc = session.docs.first(where: { $0.docId == docId })
        else { return nil }
        return doc.name ?? String(localized: "Shared")
    }

    /// A typed call's `workspace` for a doc: nil for the Personal
    /// workspace (and for a row that names none).
    private func selector(_ docId: String?) -> String? {
        docId == session?.primaryDocId ? nil : docId
    }

    /// `Engine.item`: one item's page.
    func refreshItem(_ ref: ItemRef) {
        let (workspace, id) = (selector(ref.docId), ref.itemId)
        read({ try $0.item(workspace: workspace, id: id) }) { [weak self] item, error in
            if let item { self?.itemDetails[ref.key] = item }
            self?.itemErrors[ref.key] = error
        }
    }

    /// The Home index: every open workspace's lists, one fetch per
    /// doc. Docs no longer open drop out.
    func refreshLists() {
        guard let session else { return }
        let open = Set(session.docs.map(\.docId))
        listsByDoc = listsByDoc.filter { open.contains($0.key) }
        browsedLists = browsedLists.filter { open.contains($0.value.docId) }
        refreshListRows()
    }

    /// Each open workspace's list rows alone (names, icons, open
    /// counts). Also run after a write of ours, so the counts on
    /// Home's rows follow it.
    private func refreshListRows() {
        for doc in session?.docs ?? [] {
            let workspace = selector(doc.docId)
            read({ try $0.lists(workspace: workspace, filter: .all) }) { [weak self] rows, _ in
                guard let self else { return }
                if let rows { listsByDoc[doc.docId] = rows }
                // A list or workspace the Done filter named is gone
                // (or back): the rows on screen answer another filter.
                if let fetched = doneFetchedFilter, fetched != doneFilter { refreshDone() }
            }
        }
    }

    /// `Engine.items`: one browsed list's Open items.
    func refreshList(_ ref: ListRef) {
        browsedLists[ref.key] = ref
        let workspace = selector(ref.docId)
        let listId = ref.listId
        read({ try $0.items(workspace: workspace, list: listId, includeDone: false) }) { [weak self] rows, error in
            if let rows { self?.listItems[ref.key] = rows }
            self?.listErrors[ref.key] = error
        }
    }

    /// The loaded window came back full, so older items may follow.
    var doneHasMore: Bool { doneLoadedLimit > 0 && doneCount >= doneLoadedLimit }

    /// `Engine.doneEverywhere` and `Engine.doneCountEverywhere`. A
    /// reply for a window that has since grown, or for a filter that
    /// has since changed, is dropped: the newer read is already on its
    /// way.
    func refreshDone() {
        let limit = doneLimit
        let filter = doneFilter
        doneFetchedFilter = filter
        read({ try $0.doneEverywhere(filter: filter, limit: UInt32(limit)) }) { [weak self] rows, error in
            guard let self, limit == doneLimit, filter == doneFetchedFilter else { return }
            if let rows {
                doneDays = DoneDay.days(from: rows)
                doneCount = rows.count
                doneLoadedLimit = limit
            }
            doneError = error
        }
        read({ try $0.doneCountEverywhere(filter: filter) }) { [weak self] total, _ in
            guard let self, filter == doneFetchedFilter else { return }
            if let total { doneTotal = Int(total) }
        }
    }

    /// The Done filter in effect: the choice narrowed to the workspaces
    /// that are open and the lists on offer, as the web's
    /// `effectiveDoneFilter` has it (a list of an unchecked workspace
    /// does not empty the page from behind a sheet that no longer shows
    /// it). An id is dropped only once it is known gone: before the
    /// session and a workspace's lists are read the choice stands, so a
    /// launch does not show the unfiltered page first.
    var doneFilter: DoneFilter {
        guard let session else { return doneFilterChoice }
        let open = session.docs.map(\.docId)
        let docs = doneFilterChoice.docs.filter { open.contains($0) }
        let lists = doneFilterChoice.lists.filter { key in
            guard let docId = open.first(where: { key.hasPrefix($0 + "/") }),
                  docs.isEmpty || docs.contains(docId)
            else { return false }
            guard let rows = listsByDoc[docId] else { return true }
            let listId = key.dropFirst(docId.count + 1)
            return listId == "inbox" || rows.contains { $0.id == listId }
        }
        return DoneFilter(docs: docs, lists: lists)
    }

    /// The workspaces the Done filter offers: every open one, Personal
    /// first.
    var doneWorkspaceOptions: [DoneWorkspaceOption] {
        (session?.docs ?? []).map {
            DoneWorkspaceOption(
                docId: $0.docId,
                name: $0.home ? String(localized: "Personal") : $0.name ?? String(localized: "Shared"),
                shared: !$0.home)
        }
    }

    /// The lists the Done filter offers: those of the checked
    /// workspaces, or of every one when none is checked; each
    /// workspace's Inbox, then its lists in their order, archived ones
    /// included (their finished items are still on the page).
    var doneListOptions: [DoneListOption] {
        let docs = doneFilter.docs
        return (session?.docs ?? [])
            .filter { docs.isEmpty || docs.contains($0.docId) }
            .flatMap { doc -> [DoneListOption] in
                let workspace = sharedWorkspaceName(doc.docId)
                let inbox = DoneListOption(
                    docId: doc.docId, listId: "inbox", name: String(localized: "Inbox"),
                    icon: nil, workspace: workspace, archived: false)
                return [inbox] + (listsByDoc[doc.docId] ?? []).filter { $0.id != "inbox" }.map {
                    DoneListOption(
                        docId: doc.docId, listId: $0.id, name: $0.name,
                        icon: $0.icon, workspace: workspace, archived: $0.archivedAt != nil)
                }
            }
    }

    // MARK: Calendar filter (`AgendaView.filterMenu`)

    /// Check or uncheck a workspace in the Calendar filter.
    func toggleCalendarWorkspace(_ docId: String) {
        var checked = calendarCheckedDocs
        checked.toggle(docId)
        setCalendarCheckedDocs(checked)
    }

    func clearCalendarFilter() {
        setCalendarCheckedDocs([])
    }

    /// Make `checked` the filter's checked workspaces
    /// (`Engine.setCalendarCheckedDocs`, which works out the hidden
    /// set), shown at once, then the agenda read again.
    private func setCalendarCheckedDocs(_ checked: [String]) {
        guard checked != calendarCheckedDocs else { return }
        calendarCheckedDocs = checked
        call({ try $0.setCalendarCheckedDocs(checked: checked) }) { [weak self] result in
            guard let self else { return }
            if case .failure(let error) = result {
                lastError = String(localized: "could not update items: \(engineErrorMessage(error))")
            }
            refreshAgenda()
        }
    }

    /// Check or uncheck a workspace in the Done filter.
    func toggleDoneWorkspace(_ docId: String) {
        editDoneFilter { $0.docs.toggle(docId) }
    }

    /// Check or uncheck a list (`DoneListOption.key`) in the Done filter.
    func toggleDoneList(_ key: String) {
        editDoneFilter { $0.lists.toggle(key) }
    }

    func clearDoneFilter() {
        editDoneFilter { $0 = DoneFilter(docs: [], lists: []) }
    }

    /// An edit starts from the filter in effect, so a toggle also drops
    /// the stale ids (a closed workspace, a deleted list) the choice
    /// was carrying. The window starts over: what was scrolled to
    /// belonged to the filter before.
    private func editDoneFilter(_ edit: (inout DoneFilter) -> Void) {
        var next = doneFilter
        edit(&next)
        doneFilterChoice = next
        DoneFilterStore.save(next)
        doneLimit = AppModel.donePage
        refreshDone()
    }

    /// The end of the Done list scrolled into sight: double the window
    /// and refetch it whole. Refetching the top N stays consistent while
    /// items are ticked and reopened underneath, where a cursor would
    /// skip or repeat rows; doubling keeps the total refetched linear in
    /// how far the user scrolls. A no-op while a larger window is
    /// already being fetched.
    func loadMoreDone() {
        guard doneHasMore, doneLoadedLimit == doneLimit else { return }
        doneLimit *= 2
        refreshDone()
    }

    /// `Engine.assigned`.
    func refreshAssigned() {
        read({ try $0.assigned() }) { [weak self] rows, error in
            if let rows { self?.assigned = rows }
            self?.assignedError = error
        }
    }

    /// `Engine.inbox`, narrowed to the Inbox page's filter as stored
    /// (`inboxWorkspaceKey`): the engine picks the workspaces and
    /// merges their rows.
    func refreshInbox() {
        let filter = Self.inboxFilterIds(UserDefaults.standard.string(forKey: inboxWorkspaceKey) ?? "")
        read({ try $0.inbox(filter: filter) }) { [weak self] rows, error in
            if let rows { self?.inbox = rows }
            self?.inboxError = error
        }
    }

    /// The workspace an Inbox row is in: its doc id, Personal's when
    /// the row names none.
    func docId(of entry: InboxEntry) -> String {
        entry.docId ?? session?.primaryDocId ?? ""
    }

    /// The Inbox page's workspace filter as stored (`inboxWorkspaceKey`):
    /// the picked doc ids, comma-separated; empty is every workspace.
    static func inboxFilterIds(_ stored: String) -> [String] {
        stored.split(separator: ",").map(String.init).filter { !$0.isEmpty }
    }

    static func inboxFilterStored(_ ids: [String]) -> String {
        ids.joined(separator: ",")
    }

    /// The open workspaces the stored filter picks; none is All
    /// Workspaces. A pick of a closed workspace is ignored, as the
    /// engine ignores it.
    func inboxPicked(_ stored: String) -> [String] {
        let open = Set((session?.docs ?? []).map(\.docId))
        return Self.inboxFilterIds(stored).filter(open.contains)
    }

    /// The workspaces the Inbox shows, Personal first: the picked ones,
    /// or every one (the engine's rule, `monoplan_core::inbox::scope`).
    func inboxShown(_ stored: String) -> [String] {
        let picked = inboxPicked(stored)
        let open = (session?.docs ?? []).map(\.docId)
        return picked.isEmpty ? open : open.filter(picked.contains)
    }

    /// The stored filter with one workspace checked or unchecked. A
    /// pick of a closed workspace is kept; picking every open one is
    /// All Workspaces.
    func inboxFilterToggling(_ docId: String, in stored: String) -> String {
        var ids = Self.inboxFilterIds(stored)
        if let at = ids.firstIndex(of: docId) { ids.remove(at: at) } else { ids.append(docId) }
        let open = (session?.docs ?? []).map(\.docId)
        return !open.isEmpty && open.allSatisfy(ids.contains) ? "" : Self.inboxFilterStored(ids)
    }

    /// Where a capture on the Inbox page goes: Personal's Inbox while
    /// Personal is shown, else the first shown workspace's, so the new
    /// row is seen.
    func inboxCaptureList(_ stored: String) -> ListRef? {
        let shown = inboxShown(stored)
        let home = session?.primaryDocId
        let homeShown = home.map { shown.isEmpty || shown.contains($0) } ?? false
        guard let docId = homeShown ? home : shown.first, !docId.isEmpty else { return nil }
        return ListRef(docId: docId, listId: "inbox", name: String(localized: "Inbox"))
    }

    /// `Engine.reminders`, handed to the scheduler: the notifications
    /// pending on this iPhone follow every change to what is timed
    /// (spec/reminders.md).
    func refreshReminders() {
        read({ try $0.reminders() }) { [weak self] rows, _ in
            guard let rows else { return }
            self?.reminders.update(rows.map {
                ReminderSource(
                    docId: $0.docId, itemId: $0.id, listId: $0.listId, title: $0.text, when: $0.when,
                    event: $0.event)
            })
        }
        refreshPlaceAlerts()
    }

    /// The open tasks with a place on the map (`Engine.placedItems`,
    /// events left out), handed to the scheduler with where the phone
    /// last was: the regions this iPhone alerts on arriving in follow
    /// every change to them (spec/place-alerts.md). None while the
    /// alerts are off or the app may not use precise location: a
    /// region is watched only with it.
    private func refreshPlaceAlerts() {
        guard PlaceAlertSettings.isOn(), placeLocation.access == .allowed else {
            reminders.updatePlaces([], origin: nil)
            return
        }
        read({ try $0.placedItems(closed: false) }) { [weak self] rows, _ in
            guard let self, let rows else { return }
            let sources = rows.compactMap { row -> PlaceAlertSource? in
                guard !row.item.event, let place = row.item.place, let lat = place.lat, let lon = place.lon
                else { return nil }
                return PlaceAlertSource(
                    docId: row.docId, itemId: row.item.id, listId: row.item.listId, title: row.item.text,
                    label: place.label, at: GeoPoint(lat: lat, lon: lon), createdAt: row.item.createdAt)
            }
            reminders.updatePlaces(sources, origin: placeLocation.origin)
        }
    }

    /// A reminder setting changed (Settings).
    func remindersSettingsChanged() { reminders.reschedule(settingsChanged: true) }

    /// The place alerts were turned on or off (Settings): on, the
    /// moment to ask for the phone's location and for notifications.
    func placeAlertsSettingChanged() {
        if PlaceAlertSettings.isOn() { placeLocation.locate() }
        refreshPlaceAlerts()
        reminders.reschedule(settingsChanged: true)
    }

    func showedPlaces() { tappedPlaces = false }

    private func openReminder(_ ref: ItemRef) {
        if session == nil { awaitedReminder = ref } else { tappedReminder = ref }
    }

    func showedReminder() { tappedReminder = nil }

    /// `Engine.starred`.
    func refreshStarred() {
        read({ try $0.starred() }) { [weak self] rows, error in
            if let rows { self?.starred = rows }
            self?.starredError = error
        }
    }

    /// `Engine.placedItems`: the Open items, and with `closed` the
    /// closed ones as well.
    func refreshPlaced(closed: Bool) {
        read({ try $0.placedItems(closed: closed) }) { [weak self] rows, error in
            if let rows { self?.placed = rows }
            self?.placedError = error
        }
    }

    /// How far ahead the widget's agenda reaches: far enough to be the
    /// whole future, as the Calendar page's (`Engine.calendar`).
    static let widgetAgendaDays: UInt32 = 36_500

    /// The Calendar page's device preferences, for `Engine.calendar`:
    /// its filter menu's "Hide Empty Days" and "Hide Past Days".
    static var calendarOptions: CalendarOptions {
        let defaults = UserDefaults.standard
        return CalendarOptions(
            hideEmptyDays: defaults.bool(forKey: agendaHideEmptyDaysKey),
            hidePastDays: defaults.bool(forKey: agendaHidePastDaysKey))
    }

    /// `Engine.calendar`: the Calendar page's days, every workspace
    /// merged (those hidden from the calendar left out), shaped by the
    /// engine: the days gone by that something was on, Overdue, Today,
    /// the days ahead (the empty ones per `calendarOptions`), a span on
    /// each of its days. The plain agenda from today on goes to the
    /// widget as JSON (`Engine.agendaJson`), which decodes it in its
    /// own process.
    func refreshAgenda() {
        let days = Self.widgetAgendaDays
        let options = Self.calendarOptions
        read({
            (
                groups: try $0.calendar(options: options), json: try $0.agendaJson(days: days),
                count: Int(try $0.calendarCount()), checked: try $0.calendarCheckedDocs()
            )
        }) { [weak self] read, error in
            if let read {
                self?.agenda = read.groups
                self?.calendarCount = read.count
                self?.calendarCheckedDocs = read.checked
                Self.leaveWidgetSnapshot(read.json)
            }
            self?.agendaError = error
        }
    }

    /// Leave the home-screen widget what was just read (`AgendaSnapshot`
    /// in the Kit: the widget runs no engine, it draws this file), and
    /// have it redraw when that changed. Nothing to do in a build
    /// without the app group.
    private static func leaveWidgetSnapshot(_ json: String) {
        guard let url = AgendaSnapshot.url() else { return }
        if (try? AgendaSnapshot.write(json, to: url)) == true {
            WidgetCenter.shared.reloadTimelines(ofKind: AgendaSnapshot.widgetKind)
        }
    }

    /// Signed out: the widget's copy of the agenda goes too.
    private static func clearWidgetSnapshot() {
        guard let url = AgendaSnapshot.url() else { return }
        if AgendaSnapshot.clear(at: url) {
            WidgetCenter.shared.reloadTimelines(ofKind: AgendaSnapshot.widgetKind)
        }
    }

    /// `Engine.account` and `Engine.workspaces`, read together: who is
    /// signed in and which workspaces are open. The lists and the bin
    /// wait for it: it names the docs to ask.
    func loadSession() {
        guard engineController.calls != nil else {
            session = nil
            return
        }
        read({ Session(account: try $0.account(), docs: try $0.workspaces()) }) { [weak self] session, _ in
            guard let self, let session else { return }
            self.session = session
            if let ref = awaitedReminder {
                awaitedReminder = nil
                tappedReminder = ref
            }
            refreshLists()
            refreshRoles()
            if !binLoaded { refreshBin() }
        }
    }

    /// Push now and report whether the server answers (`Engine.sync`).
    /// The answer itself arrives as the engine's next sync report.
    func syncNow() {
        call({ _ = try $0.sync() }) { [weak self] result in
            if case .failure(let error) = result { self?.lastError = engineErrorMessage(error) }
        }
    }

    /// Drop the socket and dial again now (`Engine.reconnect`).
    func reconnect() {
        call({ _ = try $0.reconnect() }) { [weak self] result in
            if case .failure(let error) = result { self?.lastError = engineErrorMessage(error) }
        }
    }

    func signIn(_ creds: SignInCredentials, completion: @escaping (Bool) -> Void) {
        signingIn = true
        signInError = nil
        busy = "Signing in"
        engineController.signIn(creds) { [weak self] error in
            guard let self else { return }
            signingIn = false
            busy = nil
            hasAccount = location.hasAccount
            if let error {
                signInError = error.localizedDescription
                completion(false)
            } else {
                completion(true)
            }
        }
    }

    /// Create an account. The completion gets the recovery code, for
    /// the sheet to show once; it is kept nowhere else. Nil on failure
    /// (`signInError` says why).
    func signUp(_ creds: SignInCredentials, completion: @escaping (String?) -> Void) {
        signingIn = true
        signInError = nil
        busy = "Creating account"
        engineController.signUp(creds) { [weak self] result in
            guard let self else { return }
            signingIn = false
            busy = nil
            hasAccount = location.hasAccount
            switch result {
            case .success(let outcome):
                completion(outcome.recoveryCode)
            case .failure(let error):
                signInError = error.localizedDescription
                completion(nil)
            }
        }
    }

    func signOut() {
        busy = "Signing out"
        engineController.signOut { [weak self] error in
            guard let self else { return }
            busy = nil
            hasAccount = location.hasAccount
            if let error { lastError = error.localizedDescription }
            if !hasAccount { Self.clearWidgetSnapshot() }
        }
    }

    /// The delete sheet's submit. The password goes straight to Rust,
    /// which re-verifies it with the server; a wrong one changes
    /// nothing. On success the profile is wiped, as after Sign Out.
    /// The completion says whether the account is gone (the sheet then
    /// closes itself); `deleteAccountError` says why not.
    func deleteAccount(password: String, completion: @escaping (Bool) -> Void) {
        deletingAccount = true
        deleteAccountError = nil
        busy = "Deleting account"
        engineController.deleteAccount(password: password) { [weak self] error in
            guard let self else { return }
            deletingAccount = false
            busy = nil
            hasAccount = location.hasAccount
            deleteAccountError = error?.localizedDescription
            if !hasAccount { Self.clearWidgetSnapshot() }
            completion(error == nil)
        }
    }

    /// Back in the foreground: the socket is dead after a suspension
    /// of any length. A local-only workspace has nothing to dial.
    func didBecomeActive() {
        // Time has passed: reminders fired, and the furthest ones the
        // pending set had no room for may now fit. The phone may have
        // moved too: which regions are nearest is asked again.
        if PlaceAlertSettings.isOn() { placeLocation.locate() }
        refreshReminders()
        guard let sync, !sync.localOnly else { return }
        reconnect()
    }

    /// Clean exit is not a thing on iOS (the process is killed, not
    /// quit); the engine persists on its own tick and the WAL covers the
    /// rest. Kept for completeness and tests.
    func stopEngine() { engineController.stop() }
}
