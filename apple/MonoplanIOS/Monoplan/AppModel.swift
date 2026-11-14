// The one observable the views read: the client's state (what the
// engine says about sync), the engine's identity, the session (account,
// server, open docs) and the in-flight account work. It owns the
// `EngineClient` and the `EngineController`, exactly as the macOS app's
// delegate does; the views never touch either.

import Combine
import Foundation
import MonoplanCore
import MonoplanMacKit
import WidgetKit

/// One binned item and the workspace it is in.
struct BinnedItem: Equatable, Identifiable {
    let docId: String
    let row: BinnedRow

    var id: String { "\(docId)/\(row.id)" }
    var ref: ItemRef { ItemRef(docId: docId, itemId: row.id, title: row.text) }
}

final class AppModel: ObservableObject {
    /// Connected with the engine's current sync status, or why not yet.
    @Published private(set) var clientState: EngineClient.State = .connecting("starting")
    /// What the engine said in its hello: its build id and pid.
    @Published private(set) var engineInfo: EngineClient.EngineInfo?
    /// Identity, server and open docs, from `Engine.session`; nil without
    /// an engine.
    @Published private(set) var session: EngineSession?
    /// The profile on disk carries a device token.
    @Published private(set) var hasAccount = false
    /// A sign-in or sign-out in progress.
    @Published private(set) var busy: String?
    /// `Changed` events since the client connected, and the last one's
    /// time: the cheapest proof that remote ops are arriving.
    @Published private(set) var changeCount = 0
    @Published private(set) var lastChangeAt: Date?
    /// The agenda's day groups (`agenda --days <agendaDays> --json`),
    /// refetched on `Changed`.
    @Published private(set) var agenda: [AgendaDay] = []
    @Published private(set) var agendaError: String?
    /// The Starred lens (`starred --all --json`): Home's curated order
    /// with its cross-workspace entries resolved.
    @Published private(set) var starred: [FocusRow] = []
    @Published private(set) var starredError: String?
    /// The Open items that carry a place, every workspace (`places
    /// --items --json`): what the Map page shows. Fetched while that
    /// page is up.
    @Published private(set) var placed: [PlacedItem] = []
    @Published private(set) var placedError: String?
    /// The Done lens (`done -n <limit> --json`): the Personal
    /// workspace's newest `doneLimit` Done items, grouped by the day
    /// each was ticked. A window, never the whole history: the view
    /// grows it as the end scrolls into sight (`loadMoreDone`), and a
    /// `Changed` refetches only as far as has been scrolled.
    @Published private(set) var doneDays: [DoneDay] = []
    @Published private(set) var doneError: String?
    /// The window `doneDays` was fetched with, and how many rows came
    /// back: a full window means there may be more.
    @Published private(set) var doneLoadedLimit = 0
    private var doneCount = 0
    /// How many Done items there are in all (`done --count`), loaded or
    /// not; nil until first read.
    @Published private(set) var doneTotal: Int?
    private var doneLimit = AppModel.donePage
    static let donePage = 50
    /// The bin (`bin show --json` per open workspace), newest binned
    /// first. Fetched with the other lenses: Home shows its Bin row
    /// only while there is something in it.
    @Published private(set) var binned: [BinnedItem] = []
    @Published private(set) var binError: String?
    /// The bin has been read at least once.
    @Published private(set) var binLoaded = false
    /// Counts bin fetches, so replies to a superseded round are dropped.
    private var binRound = 0
    /// List id → display name, for the rows of lenses that span lists.
    @Published private(set) var listNames = Lists.names(from: [])
    /// Each open workspace's lists (`list ls --json --all` per doc),
    /// keyed by doc id: the Home index. Refetched with the session.
    @Published private(set) var listsByDoc: [String: [ListRow]] = [:]
    /// A browsed list's Open items, keyed by `ListRef.key`, fetched when
    /// its page appears and again on each `Changed` while it is shown.
    @Published private(set) var listItems: [String: [ItemRow]] = [:]
    @Published private(set) var listErrors: [String: String] = [:]
    /// The lists opened so far, to reread after a write of ours.
    private var browsedLists: [String: ListRef] = [:]
    /// An opened item (`item show --json`), keyed by `ItemRef.key`,
    /// fetched when its page appears and on each `Changed` while shown.
    @Published private(set) var itemDetails: [String: ItemDetail] = [:]
    @Published private(set) var itemErrors: [String: String] = [:]
    /// Search results (`find <query> --json`: every open workspace,
    /// ranked), and the query they answer. The hits of the query before
    /// stay until the next reply, so `searchedQuery` says whether they
    /// are for what is typed now.
    @Published private(set) var searchHits: [SearchHit] = []
    @Published private(set) var searchedQuery = ""
    @Published private(set) var searchError: String?
    /// What is being searched for, trimmed; empty while search is idle.
    private var searchQuery = ""
    static let searchLimit = 50
    private var refreshQueued = false
    @Published var lastError: String?
    @Published var signingIn = false
    @Published var signInError: String?

    let location: ProfileLocation
    private let client: EngineClient
    private let engineController: EngineController

    init() {
        location = ProfileLocation.resolve()
        // Application Support does not exist in a fresh iOS container;
        // the Rust side creates the profile dir itself but not its
        // parents.
        try? FileManager.default.createDirectory(at: location.dir, withIntermediateDirectories: true)
        client = EngineClient(clientId: "monoplan-ios")
        engineController = EngineController(location: location, client: client)
        hasAccount = location.hasAccount
        client.onState = { [weak self] state in self?.stateChanged(state) }
        client.onEvent = { [weak self] event in self?.eventArrived(event) }
        client.start()
    }

    /// The sync status while connected.
    var sync: SyncStatus? {
        if case .connected(let sync) = clientState { return sync }
        return nil
    }

    private func stateChanged(_ state: EngineClient.State) {
        clientState = state
        engineInfo = client.engine
        hasAccount = location.hasAccount
        if case .connected = state {
            loadSession()
            refreshAgenda()
            refreshStarred()
            refreshDone()
        } else {
            session = nil
        }
    }

    private func eventArrived(_ event: WatchEvent) {
        if case .changed = event {
            changeCount += 1
            lastChangeAt = Date()
            scheduleRefresh()
        }
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
        refreshDone()
        refreshSearch()
        refreshBin()
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
        fetch(.find(query: query, limit: Self.searchLimit), what: "search results", parse: Find.parse) { [weak self] hits, error in
            guard let self, query == searchQuery else { return }
            if let hits {
                searchHits = hits
                searchedQuery = query
            }
            searchError = error
        }
    }

    /// The item a capture last created and the list it landed in, until
    /// that list's page has shown it: Home goes to the list, and the
    /// list scrolls to the item and takes this back (`showedCaptured`).
    @Published private(set) var captured: Captured?

    struct Captured: Equatable {
        let list: ListRef
        let itemId: String
    }

    /// The list page has the captured item on screen.
    func showedCaptured() { captured = nil }

    /// Capture: create an item at the top of `list`, in Backlog
    /// (`item add <text> --list --top`), then reread what is on screen as
    /// `setDone` does. `done` gets the new item, or why there is none;
    /// it may be left out by a caller that is going away (a capture
    /// sheet closing over a typed title).
    func addItem(
        text: String, to list: ListRef, whenSuggested: String? = nil,
        done: ((ItemRef?, String?) -> Void)? = nil
    ) {
        client.send(.addItem(text: text, list: list.listId, top: true, workspace: selector(list.docId))) { [weak self] result in
            guard let self else { return }
            let failure: String
            switch result {
            case .success(.output(let stdout, _)):
                let id = stdout.trimmingCharacters(in: .whitespacesAndNewlines)
                if !id.isEmpty {
                    refreshLenses()
                    refreshListRows()
                    for list in browsedLists.values { refreshList(list) }
                    let ref = ItemRef(docId: list.docId, itemId: id, title: text)
                    captured = Captured(list: list, itemId: id)
                    // The date its title named, as a question on the
                    // new item (`WhenSuggest`).
                    if let whenSuggested { setWhenSuggested(ref, when: whenSuggested) }
                    done?(ref, nil)
                    return
                }
                failure = "could not add item: no id came back"
            case .success(let other):
                failure = "could not add item: \(String(describing: other))"
            case .failure(let error):
                failure = "could not add item: \(engineErrorMessage(error))"
            }
            lastError = failure
            done?(nil, failure)
        }
    }

    /// Tick an item, or reopen a closed one (`item set --lifecycle`).
    /// Nothing is assumed about the result: once the engine has applied
    /// it, everything on screen is read again at once rather than after
    /// the `Changed` coalescing delay, so the tick lands in the same
    /// frame or two. In process, the write and the reads are
    /// milliseconds.
    func setDone(_ ref: ItemRef, done: Bool) {
        write(.setDone(id: ref.itemId, done: done, workspace: selector(ref.docId)), to: ref)
    }

    /// Move an item to any workflow state, by its JSON name
    /// (`item set --lifecycle <state>`), then reread as `setDone` does.
    func setLifecycle(_ ref: ItemRef, state: String) {
        write(.setLifecycle(id: ref.itemId, state: state, workspace: selector(ref.docId)), to: ref)
    }

    /// Retitle an item (`item set --text`), then reread as `setDone`
    /// does.
    func setText(_ ref: ItemRef, text: String, suggestion: SuggestionEdit = .keep) {
        write(.setText(id: ref.itemId, text: text, suggestion: suggestion, workspace: selector(ref.docId)), to: ref)
    }

    /// Raise a date suggestion on an item, or dismiss the pending one
    /// (nil: the user's "no"). The "yes" is `setWhen` with the
    /// suggested value: any date set clears the suggestion.
    func setWhenSuggested(_ ref: ItemRef, when: String?) {
        write(.setWhenSuggested(id: ref.itemId, when: when, workspace: selector(ref.docId)), to: ref)
    }

    /// Move an item to another list of its own workspace
    /// (`item set --list`), then reread as `setDone` does.
    func moveItem(_ ref: ItemRef, toList listId: String) {
        write(.moveItem(id: ref.itemId, list: listId, workspace: selector(ref.docId)), to: ref)
    }

    /// Star an item (to the top of Starred) or unstar it
    /// (`item set --star 1` / `--unstar`). The ref is the Home doc's
    /// whichever workspace the item is in; the engine sees to that.
    func setStarred(_ ref: ItemRef, starred: Bool) {
        write(.setStarred(id: ref.itemId, starred: starred, workspace: selector(ref.docId)), to: ref)
    }

    /// Move an item to the bin, or restore it from there
    /// (`item bin` / `item restore`), then reread as `setDone` does.
    func setBinned(_ ref: ItemRef, binned: Bool) {
        write(.setBinned(id: ref.itemId, binned: binned, workspace: selector(ref.docId)), to: ref)
    }

    /// Copy an item beside itself in its own list (`item duplicate`),
    /// then reread as `setDone` does. The copy's id is not needed: the
    /// reread shows it. `star` also stars the copy of a starred item,
    /// directly below it in Starred (`--star`).
    func duplicateItem(_ ref: ItemRef, star: Bool = false) {
        write(.duplicateItem(id: ref.itemId, star: star, workspace: selector(ref.docId)), to: ref)
    }

    /// Hard-delete a binned item (`bin rm`). Its page is forgotten
    /// first, so nothing rereads an item that is no longer there.
    func deleteBinned(_ ref: ItemRef) {
        itemDetails[ref.key] = nil
        write(.deleteBinned(id: ref.itemId, workspace: selector(ref.docId)), to: ref)
    }

    /// Hard-delete everything in the bin, a workspace at a time
    /// (`bin empty`), then reread as `setDone` does. Only the
    /// workspaces with something binned are asked.
    func emptyBin() {
        let docs = Set(binned.map(\.docId))
        for docId in docs {
            for item in binned where item.docId == docId {
                itemDetails[item.ref.key] = nil
            }
            client.send(.emptyBin(workspace: selector(docId))) { [weak self] result in
                guard let self else { return }
                switch result {
                case .success(.output):
                    refreshLenses()
                case .success(let other):
                    lastError = "could not empty the bin: \(String(describing: other))"
                case .failure(let error):
                    lastError = "could not empty the bin: \(engineErrorMessage(error))"
                }
            }
        }
    }

    /// `bin show --json` for each open workspace, merged newest binned
    /// first. A workspace that fails keeps the others' rows and leaves
    /// its error.
    func refreshBin() {
        guard let session else { return }
        binRound += 1
        let round = binRound
        var rows: [BinnedItem] = []
        var failure: String?
        var waiting = session.docs.count
        if waiting == 0 {
            binned = []
            binLoaded = true
            return
        }
        for doc in session.docs {
            let docId = doc.docId
            client.send(.bin(workspace: selector(docId))) { [weak self] result in
                guard let self, round == binRound else { return }
                switch result {
                case .success(.output(let stdout, _)):
                    do {
                        rows += try Bin.parse(stdout).map { BinnedItem(docId: docId, row: $0) }
                    } catch {
                        failure = "could not read the bin: \(error.localizedDescription)"
                    }
                case .success(let other):
                    failure = "unexpected reply: \(String(describing: other))"
                case .failure(let error):
                    // No engine is not an error (as in `fetch`): the
                    // next one's refresh replaces the rows.
                    if case ClientError.notConnected = error { return }
                    if case ClientError.engineStopped = error { return }
                    if case EngineError.Stopped = error { return }
                    failure = engineErrorMessage(error)
                }
                waiting -= 1
                if waiting == 0 {
                    binned = rows.sorted {
                        ($0.row.binnedAt, $0.id) > ($1.row.binnedAt, $1.id)
                    }
                    binError = failure
                    binLoaded = true
                }
            }
        }
    }

    /// The name of a list in any open workspace, for a row that names
    /// the list its item is in.
    func listName(_ listId: String, inDoc docId: String) -> String? {
        Lists.names(from: listsByDoc[docId] ?? [])[listId]
    }

    /// Set or clear (nil) an item's planned date, with what to do to
    /// its duration in the same call (`item set --when [--duration]`).
    func setWhen(_ ref: ItemRef, when: String?, duration: DurationEdit = .keep) {
        write(.setWhen(id: ref.itemId, when: when, duration: duration, workspace: selector(ref.docId)), to: ref)
    }

    /// Set or clear (nil) an item's duration in minutes.
    func setDuration(_ ref: ItemRef, minutes: Int?) {
        write(.setDuration(id: ref.itemId, minutes: minutes, workspace: selector(ref.docId)), to: ref)
    }

    /// Set an item's place, or clear it (nil). `exact` writes a bare
    /// label as just that, instead of letting the engine take a known
    /// place of the same label (`item set --place`).
    func setPlace(_ ref: ItemRef, place: PlaceInfo?, exact: Bool = false) {
        write(.setPlace(id: ref.itemId, place: place, exact: exact, workspace: selector(ref.docId)), to: ref)
    }

    /// The account's known places matching `query` (all when empty),
    /// most recently used first: what the place editor offers before
    /// any lookup leaves the device. Nothing comes back on failure.
    func knownPlaces(matching query: String, limit: Int = 8, completion: @escaping ([KnownPlace]) -> Void) {
        fetch(.places(query: query, limit: limit), what: "places", parse: KnownPlace.parse) { rows, _ in
            if let rows { completion(rows) }
        }
    }

    /// The lists an item of this workspace can be in: its Inbox, then
    /// its active lists in the doc's order.
    func lists(inDoc docId: String?) -> [ListRef] {
        guard let docId = docId ?? session?.primaryDocId else { return [] }
        return [ListRef(docId: docId, listId: "inbox", name: "Inbox")]
            + Lists.active(listsByDoc[docId] ?? []).map {
                ListRef(docId: docId, listId: $0.id, name: $0.name, icon: $0.icon)
            }
    }

    /// Create a list at the end of a workspace's lists (`list add
    /// <name>`), then reread Home's list rows. `done` gets the new
    /// list, or why there is none.
    func addList(name: String, inDoc docId: String, done: @escaping (ListRef?, String?) -> Void) {
        let name = name.trimmingCharacters(in: .whitespacesAndNewlines)
        client.send(.addList(name: name, workspace: selector(docId))) { [weak self] result in
            guard let self else { return }
            let failure: String
            switch result {
            case .success(.output(let stdout, _)):
                let id = stdout.trimmingCharacters(in: .whitespacesAndNewlines)
                if !id.isEmpty {
                    refreshListRows()
                    done(ListRef(docId: docId, listId: id, name: name), nil)
                    return
                }
                failure = "could not add list: no id came back"
            case .success(let other):
                failure = "could not add list: \(String(describing: other))"
            case .failure(let error):
                failure = "could not add list: \(engineErrorMessage(error))"
            }
            lastError = failure
            done(nil, failure)
        }
    }

    /// Rename a user-created list (`list rename`).
    func renameList(_ list: ListRef, to name: String) {
        writeList(.renameList(id: list.listId, name: name, workspace: selector(list.docId)))
    }

    /// Set a list's emoji, or clear it (nil) (`list icon`).
    func setListIcon(_ list: ListRef, icon: String?) {
        writeList(.setListIcon(id: list.listId, icon: icon, workspace: selector(list.docId)))
    }

    /// Archive a list (`list archive`): it leaves Home, its items
    /// kept. `done` gets why not, or nil once it is archived.
    func archiveList(_ list: ListRef, done: @escaping (String?) -> Void) {
        writeList(.archiveList(id: list.listId, workspace: selector(list.docId)), done: done)
    }

    /// A list edit, then Home's list rows reread; item rows elsewhere
    /// name their list, so the lenses are reread too.
    private func writeList(_ request: Request, done: ((String?) -> Void)? = nil) {
        client.send(request) { [weak self] result in
            guard let self else { return }
            let failure: String
            switch result {
            case .success(.output):
                refreshListRows()
                refreshLenses()
                done?(nil)
                return
            case .success(let other):
                failure = "could not update list: \(String(describing: other))"
            case .failure(let error):
                failure = "could not update list: \(engineErrorMessage(error))"
            }
            lastError = failure
            done?(failure)
        }
    }

    private func write(_ request: Request, to ref: ItemRef) {
        client.send(request) { [weak self] result in
            guard let self else { return }
            switch result {
            case .success(.output):
                refreshLenses()
                if itemDetails[ref.key] != nil { refreshItem(ref) }
                refreshListRows()
                for list in browsedLists.values { refreshList(list) }
            case .success(let other):
                lastError = "could not update item: \(String(describing: other))"
            case .failure(let error):
                lastError = "could not update item: \(engineErrorMessage(error))"
            }
        }
    }

    /// A shared workspace's name: nil for the Personal workspace (and
    /// for a doc that is not open), as the web's `isSharedDoc` has any
    /// doc but Home shared.
    func sharedWorkspaceName(_ docId: String) -> String? {
        guard let session, docId != session.primaryDocId,
              let doc = session.docs.first(where: { $0.docId == docId })
        else { return nil }
        return doc.name ?? "Shared"
    }

    /// The `-w` selector for a doc: nil for the Personal workspace, the
    /// frame every other read already sends.
    private func selector(_ docId: String?) -> String? {
        docId == session?.primaryDocId ? nil : docId
    }

    /// `item show <id> -w <doc> --json`: one item's page.
    func refreshItem(_ ref: ItemRef) {
        fetch(.item(id: ref.itemId, workspace: selector(ref.docId)), what: "item", parse: ItemDetail.parse) { [weak self] item, error in
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
        for doc in session.docs {
            // Each Inbox's Open items, for the count on its Home row:
            // read as a browsed list, so a write of ours rereads it too.
            refreshList(ListRef(docId: doc.docId, listId: "inbox", name: "Inbox"))
        }
    }

    /// Each open workspace's list rows alone (names, icons, open
    /// counts). Also run after a write of ours, so the counts on
    /// Home's rows follow it.
    private func refreshListRows() {
        for doc in session?.docs ?? [] {
            fetch(.lists(workspace: selector(doc.docId)), what: "lists", parse: Lists.parse) { [weak self] rows, _ in
                if let rows { self?.listsByDoc[doc.docId] = rows }
            }
        }
    }

    /// How many Open items a workspace's Inbox holds; nil until read.
    func inboxCount(_ docId: String) -> Int? {
        listItems["\(docId)/inbox"]?.count
    }

    /// `item ls --list <list> -w <doc> --json`: one browsed list's Open
    /// items.
    func refreshList(_ ref: ListRef) {
        browsedLists[ref.key] = ref
        fetch(.items(list: ref.listId, workspace: selector(ref.docId)), what: ref.name, parse: Items.parse) { [weak self] rows, error in
            if let rows { self?.listItems[ref.key] = rows }
            self?.listErrors[ref.key] = error
        }
    }

    /// The loaded window came back full, so older items may follow.
    var doneHasMore: Bool { doneLoadedLimit > 0 && doneCount >= doneLoadedLimit }

    /// `done -n <limit> --json` through the engine, with the list names
    /// its rows are labelled by. A reply for a window that has since
    /// grown is dropped: the larger fetch is already on its way.
    func refreshDone() {
        let limit = doneLimit
        fetch(.done(limit: limit), what: "done", parse: Items.parse) { [weak self] rows, error in
            guard let self, limit == doneLimit else { return }
            if let rows {
                doneDays = Done.days(from: rows)
                doneCount = rows.count
                doneLoadedLimit = limit
            }
            doneError = error
        }
        fetch(.doneCount, what: "done count", parse: Done.parseCount) { [weak self] total, _ in
            if let total { self?.doneTotal = total }
        }
        fetch(.lists(), what: "lists", parse: Lists.parse) { [weak self] rows, _ in
            if let rows { self?.listNames = Lists.names(from: rows) }
        }
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

    /// `starred --all --json` through the engine.
    func refreshStarred() {
        fetch(.focus(all: true), what: "starred", parse: Focus.parse) { [weak self] rows, error in
            if let rows { self?.starred = rows }
            self?.starredError = error
        }
    }

    /// `places --items --json` through the engine.
    func refreshPlaced() {
        fetch(.placedItems, what: "placed items", parse: Placed.parse) { [weak self] rows, error in
            if let rows { self?.placed = rows }
            self?.placedError = error
        }
    }

    /// How far ahead the agenda reaches: far enough to be the whole
    /// future. There is no recurrence, so dated items are a finite set,
    /// the builder walks every item whatever the horizon, and the list
    /// builds its rows lazily; a window would save nothing.
    static let agendaDays = 36_500

    /// `agenda --days <agendaDays> --all --json`: every workspace
    /// merged, Today first, then each non-empty day to the end of what
    /// is dated. Workspaces hidden from the calendar (the Home doc's
    /// setting) are already left out.
    func refreshAgenda() {
        fetch(.agenda(days: Self.agendaDays, all: true), what: "agenda", parse: { (json: $0, days: try Agenda.parse($0)) }) { [weak self] read, error in
            if let read {
                self?.agenda = read.days
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

    /// One read through the engine, decoded. No engine is not an error:
    /// the rows stay until the next engine's refresh replaces them, so
    /// `done` gets neither rows nor an error then.
    private func fetch<T>(
        _ request: Request, what: String, parse: @escaping (String) throws -> T,
        done: @escaping (T?, String?) -> Void
    ) {
        client.send(request) { result in
            switch result {
            case .success(.output(let stdout, _)):
                do {
                    done(try parse(stdout), nil)
                } catch {
                    done(nil, "could not read \(what): \(error.localizedDescription)")
                }
            case .success(let other):
                done(nil, "unexpected reply: \(String(describing: other))")
            case .failure(let error):
                if case ClientError.notConnected = error { return }
                if case ClientError.engineStopped = error { return }
                if case EngineError.Stopped = error { return }
                done(nil, engineErrorMessage(error))
            }
        }
    }

    /// `Engine.session` blocks for the loop's turn, so off the main thread.
    func loadSession() {
        guard let engine = engineController.runningEngine else {
            session = nil
            return
        }
        DispatchQueue.global(qos: .userInitiated).async { [weak self] in
            let result = try? engine.session()
            DispatchQueue.main.async {
                self?.session = result
                self?.refreshLists()
                // The first read waits for the session: it names
                // the docs to ask.
                if let self, !self.binLoaded { self.refreshBin() }
            }
        }
    }

    /// `Request::Sync`: push now and report whether the server answers.
    func syncNow() { send(.sync) }

    /// `Request::Reconnect`: drop the socket and dial again now.
    func reconnect() { send(.reconnect) }

    private func send(_ request: Request) {
        client.send(request) { [weak self] result in
            if case .failure(let error) = result {
                self?.lastError = error.localizedDescription
            }
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

    /// Back in the foreground: the socket is dead after a suspension
    /// of any length. A local-only workspace has nothing to dial.
    func didBecomeActive() {
        guard let sync, !sync.localOnly else { return }
        reconnect()
    }

    /// Clean exit is not a thing on iOS (the process is killed, not
    /// quit); the engine persists on its own tick and the WAL covers the
    /// rest. Kept for completeness and tests.
    func stopEngine() { engineController.stop() }
}
