// Place alerts (spec/place-alerts.md): which regions a device watches
// so that arriving at a place with open tasks shows a notification.
// Pure data here: the tasks that carry a place with coordinates are
// gathered into spots, spots a geofence could not tell apart are
// merged into one region, and the nearest regions are kept, up to
// what the system lets one app watch. `ReminderScheduler` hands the
// plan to the system's notification centre (iOS only).

import Foundation

/// A point on the map, WGS84 decimal degrees.
public struct GeoPoint: Equatable, Sendable {
    public let lat: Double
    public let lon: Double

    public init(lat: Double, lon: Double) {
        self.lat = lat
        self.lon = lon
    }

    /// Great-circle metres to `other`.
    public func metres(to other: GeoPoint) -> Double {
        let radians = Double.pi / 180
        let dLat = (other.lat - lat) * radians
        let dLon = (other.lon - lon) * radians
        let a = sin(dLat / 2) * sin(dLat / 2)
            + cos(lat * radians) * cos(other.lat * radians) * sin(dLon / 2) * sin(dLon / 2)
        return 6_371_000 * 2 * atan2(sqrt(a), sqrt(1 - a))
    }
}

/// The one setting, a device preference: never synced, never in a doc.
/// Off until asked for.
public enum PlaceAlertSettings {
    /// `UserDefaults` key; absent reads as off.
    public static let key = "placeAlerts"

    public static func isOn(in defaults: UserDefaults = .standard) -> Bool {
        defaults.bool(forKey: key)
    }
}

/// A task an alert may be for: an Open item that is not an event and
/// whose place has coordinates.
public struct PlaceAlertSource: Equatable, Sendable {
    public let docId: String?
    public let itemId: String
    public let listId: String
    public let title: String
    /// The place's label.
    public let label: String
    public let at: GeoPoint
    /// Unix millis: which places are newest, for when there is no
    /// location to measure from.
    public let createdAt: Int64

    public init(
        docId: String?, itemId: String, listId: String, title: String, label: String, at: GeoPoint,
        createdAt: Int64
    ) {
        self.docId = docId
        self.itemId = itemId
        self.listId = listId
        self.title = title
        self.label = label
        self.at = at
        self.createdAt = createdAt
    }
}

/// One task of an alert.
public struct PlaceAlertItem: Equatable, Sendable {
    public let docId: String?
    public let itemId: String
    public let listId: String
    public let title: String
}

/// One region to watch, and the notification arriving in it shows.
public struct PlannedPlaceAlert: Equatable, Sendable {
    public let centre: GeoPoint
    /// Metres: the base radius, grown to cover every spot merged in.
    public let radius: Double
    /// The labels of the places inside, each once.
    public let labels: [String]
    /// The tasks there, in the order they were given.
    public let items: [PlaceAlertItem]

    /// One per region: five decimals is about a metre.
    public var identifier: String { String(format: "place/%.5f,%.5f", centre.lat, centre.lon) }
}

public enum PlaceAlerts {
    /// The system watches at most 20 regions for one app.
    public static let limit = 20
    /// Metres around a place with nothing merged into it. Under about
    /// 100 the system's own accuracy is the limit.
    public static let radius = 150.0
    /// The widest a merged region may grow: past this an alert fires
    /// too far from the place to mean it.
    public static let maxRadius = 300.0

    /// The regions to watch: a region a spot, or several spots within
    /// `radius` of each other while the region that covers them stays
    /// within `maxRadius`. Nearest `origin` first, the newest places
    /// first without one, up to `limit`.
    public static func plan(
        _ sources: [PlaceAlertSource], origin: GeoPoint?, limit: Int = PlaceAlerts.limit,
        radius: Double = PlaceAlerts.radius, maxRadius: Double = PlaceAlerts.maxRadius
    ) -> [PlannedPlaceAlert] {
        // Tasks at the same coordinates are one spot.
        var keys: [String] = []
        var spots: [String: Spot] = [:]
        for source in sources {
            let key = String(format: "%.5f,%.5f", source.at.lat, source.at.lon)
            if spots[key] == nil {
                keys.append(key)
                spots[key] = Spot(at: source.at, sources: [])
            }
            spots[key]?.sources.append(source)
        }
        // A fixed order, so the same places always merge the same way
        // whatever order the tasks come in.
        let ordered = keys.compactMap { spots[$0] }
            .sorted { ($0.at.lat, $0.at.lon) < ($1.at.lat, $1.at.lon) }

        var clusters: [Cluster] = []
        for spot in ordered {
            var best: (index: Int, metres: Double, merged: Cluster)?
            for (index, cluster) in clusters.enumerated() {
                let metres = cluster.centre.metres(to: spot.at)
                guard metres <= radius, metres < (best?.metres ?? .infinity) else { continue }
                let merged = Cluster(spots: cluster.spots + [spot])
                if merged.reach + radius <= maxRadius { best = (index, metres, merged) }
            }
            if let best {
                clusters[best.index] = best.merged
            } else {
                clusters.append(Cluster(spots: [spot]))
            }
        }

        let alerts = clusters.map { cluster -> (alert: PlannedPlaceAlert, newest: Int64) in
            // Tasks back in the order they were given, not the spots'.
            let inside = Set(cluster.spots.flatMap(\.sources).map(Key.init))
            let members = sources.filter { inside.contains(Key($0)) }
            var labels: [String] = []
            for label in members.map(\.label) where !labels.contains(label) { labels.append(label) }
            return (
                PlannedPlaceAlert(
                    centre: cluster.centre, radius: radius + cluster.reach, labels: labels,
                    items: members.map {
                        PlaceAlertItem(docId: $0.docId, itemId: $0.itemId, listId: $0.listId, title: $0.title)
                    }),
                members.map(\.createdAt).max() ?? 0
            )
        }
        let sorted: [PlannedPlaceAlert]
        if let origin {
            sorted = alerts
                .map { (metres: origin.metres(to: $0.alert.centre), alert: $0.alert) }
                .sorted { ($0.metres, $0.alert.identifier) < ($1.metres, $1.alert.identifier) }
                .map(\.alert)
        } else {
            sorted = alerts
                .sorted { (-$0.newest, $0.alert.identifier) < (-$1.newest, $1.alert.identifier) }
                .map(\.alert)
        }
        return Array(sorted.prefix(max(0, limit)))
    }

    private struct Key: Hashable {
        let docId: String?
        let itemId: String

        init(_ source: PlaceAlertSource) {
            docId = source.docId
            itemId = source.itemId
        }
    }

    private struct Spot {
        let at: GeoPoint
        var sources: [PlaceAlertSource]
    }

    private struct Cluster {
        let spots: [Spot]
        /// The middle of the spots.
        let centre: GeoPoint
        /// Metres from the centre to the furthest spot.
        let reach: Double

        init(spots: [Spot]) {
            self.spots = spots
            let first = spots[0].at
            // Longitudes as offsets from the first spot's, so a
            // cluster across the antimeridian averages to where it is.
            let offsets = spots.map { spot -> Double in
                let delta = spot.at.lon - first.lon
                return delta > 180 ? delta - 360 : (delta < -180 ? delta + 360 : delta)
            }
            let count = Double(spots.count)
            var lon = first.lon + offsets.reduce(0, +) / count
            if lon > 180 { lon -= 360 } else if lon < -180 { lon += 360 }
            let centre = GeoPoint(lat: spots.map(\.at.lat).reduce(0, +) / count, lon: lon)
            self.centre = centre
            reach = spots.map { centre.metres(to: $0.at) }.max() ?? 0
        }
    }
}
