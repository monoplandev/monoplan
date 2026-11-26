// The item page's attachments (spec/attachments.md): the files on the
// item, under its subtasks. The Mac panel's field
// (`MonoplanMac/FilesField.swift`) for touch: a column of rows, each a
// file's glyph, name and size, over the uploads still on their way and
// a trailing "Upload file" row that offers the photo library or the
// Files picker.
//
// The bytes are online-only: a tap has the engine fetch and decrypt the
// file into a temp file, and the system's save sheet moves it to the
// place picked in Files (an app cannot write to Downloads on its own).
// It is never opened from here: the name and type are whatever a member
// of the workspace wrote. The temp file goes if the save is cancelled.
// Removing a file is permanent, so it asks first, from the "…" at a
// row's end. A binned item's files are shown and saved, and nothing is
// added or removed.

import MonoplanCore
import PhotosUI
import SwiftUI
import UniformTypeIdentifiers

struct ItemFilesField: View {
    let files: [ItemFile]
    /// Attaches under way or failed, for this item.
    let uploads: [FileUpload]
    /// A binned item's: shown, not edited.
    var locked = false
    /// There is an account to hold the bytes.
    var canAttach = true
    /// Files were picked for the item. One under
    /// `AttachmentFiles.tempURL` (a photo copied out of the library) is
    /// the model's to discard once it is sent.
    let attach: ([URL]) -> Void
    /// Fetch a file into a temp file; the result arrives on the main
    /// queue.
    let open: (ItemFile, @escaping (Result<URL, Error>) -> Void) -> Void
    let remove: (ItemFile) -> Void
    let dismissUpload: (UUID) -> Void

    /// One line's row: a full-height touch target, growing with the
    /// text (Dynamic Type).
    @ScaledMetric(relativeTo: .body) private var rowHeight: CGFloat = 44

    @State private var importing = false
    @State private var pickingPhotos = false
    @State private var photos: [PhotosPickerItem] = []
    /// The file being fetched, by id.
    @State private var opening: String?
    /// The fetched file waiting to be saved somewhere.
    @State private var fetched: URL?
    @State private var removing: ItemFile?
    /// The last open, or the last read of a picked photo, that failed.
    @State private var failure: String?

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            ForEach(files) { file in
                fileRow(file)
            }
            ForEach(uploads) { upload in
                uploadRow(upload)
            }
            if !locked, canAttach {
                attachRow
            }
        }
        .accessibilityElement(children: .contain)
        .accessibilityLabel(Text("Files"))
        .fileImporter(isPresented: $importing, allowedContentTypes: [.item], allowsMultipleSelection: true) {
            if case .success(let urls) = $0, !urls.isEmpty { attach(urls) }
        }
        .photosPicker(isPresented: $pickingPhotos, selection: $photos, matching: .images)
        .onChange(of: photos) { _, picked in
            guard !picked.isEmpty else { return }
            photos = []
            Task { await attachPhotos(picked) }
        }
        .fileMover(
            isPresented: Binding(get: { fetched != nil }, set: { if !$0 { leave() } }),
            file: fetched
        ) { _ in
            leave()
        } onCancellation: {
            leave()
        }
        .confirmationDialog(
            "Delete this file for good?",
            isPresented: Binding(get: { removing != nil }, set: { if !$0 { removing = nil } }),
            titleVisibility: .visible,
            presenting: removing
        ) { file in
            Button("Delete", role: .destructive) { remove(file) }
        } message: { file in
            Text("“\(file.name)” is removed from every device and cannot be restored.")
        }
        .alert(
            "Could not open the file",
            isPresented: Binding(get: { failure != nil }, set: { if !$0 { failure = nil } })
        ) {
            Button("OK", role: .cancel) {}
        } message: {
            Text(verbatim: failure ?? "")
        }
    }

    private func fileRow(_ file: ItemFile) -> some View {
        HStack(alignment: .center, spacing: 12) {
            Button {
                show(file)
            } label: {
                HStack(alignment: .center, spacing: 12) {
                    Image(systemName: AttachmentFiles.symbol(name: file.name))
                        .foregroundStyle(.secondary)
                        .frame(width: 24)
                    Text(verbatim: file.name)
                        .lineLimit(1)
                        .truncationMode(.middle)
                        .foregroundStyle(.primary)
                    Spacer(minLength: 8)
                    if opening == file.id {
                        ProgressView()
                    } else {
                        Text(verbatim: AttachmentFiles.sizeText(file.size))
                            .font(.footnote)
                            .foregroundStyle(.secondary)
                            .monospacedDigit()
                    }
                }
                .frame(minHeight: rowHeight)
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            if !locked {
                Menu {
                    Button("Save to Files", systemImage: "square.and.arrow.down") { show(file) }
                    Button("Delete…", systemImage: "trash", role: .destructive) { removing = file }
                } label: {
                    Image(systemName: "ellipsis")
                        .foregroundStyle(.secondary)
                        .frame(width: 32, height: rowHeight)
                        .contentShape(Rectangle())
                }
                .accessibilityLabel(Text("File actions"))
            }
        }
    }

    private func uploadRow(_ upload: FileUpload) -> some View {
        HStack(alignment: .center, spacing: 12) {
            Group {
                if upload.error == nil {
                    ProgressView()
                } else {
                    Image(systemName: "exclamationmark.triangle").foregroundStyle(.red)
                }
            }
            .frame(width: 24)
            VStack(alignment: .leading, spacing: 2) {
                Text(verbatim: upload.name)
                    .lineLimit(1)
                    .truncationMode(.middle)
                    .foregroundStyle(.secondary)
                if let error = upload.error {
                    Text(verbatim: error)
                        .font(.footnote)
                        .foregroundStyle(.red)
                        .fixedSize(horizontal: false, vertical: true)
                }
            }
            Spacer(minLength: 8)
            if upload.error != nil {
                Button {
                    dismissUpload(upload.id)
                } label: {
                    Image(systemName: "xmark")
                        .foregroundStyle(.secondary)
                        .frame(width: 32, height: rowHeight)
                        .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .accessibilityLabel(Text("Dismiss"))
            }
        }
        .frame(minHeight: rowHeight)
    }

    private var attachRow: some View {
        Menu {
            Button("Photo Library", systemImage: "photo.on.rectangle") { pickingPhotos = true }
            Button("Choose File", systemImage: "folder") { importing = true }
        } label: {
            HStack(alignment: .center, spacing: 12) {
                Image("Upload")
                    .resizable()
                    .frame(width: 18, height: 18)
                    .frame(width: 24)
                Text("Upload file")
                    .lineLimit(1)
                Spacer(minLength: 0)
            }
            .foregroundStyle(.tertiary)
            .frame(minHeight: rowHeight)
            .contentShape(Rectangle())
        }
    }

    /// The save sheet is done with the fetched file: moved, failed or
    /// cancelled. Whatever is still in the temp directory goes.
    private func leave() {
        if let fetched { AttachmentFiles.discard(fetched) }
        fetched = nil
    }

    private func show(_ file: ItemFile) {
        guard opening == nil else { return }
        opening = file.id
        open(file) { result in
            opening = nil
            switch result {
            case .success(let url): fetched = url
            case .failure(let error): failure = engineErrorMessage(error)
            }
        }
    }

    /// Copy each picked photo out of the library into a temp file and
    /// attach it. The library hands over data, not a file, and no name:
    /// it is named for the moment it was picked.
    private func attachPhotos(_ picked: [PhotosPickerItem]) async {
        let stamp = Date().formatted(.iso8601.year().month().day().time(includingFractionalSeconds: false))
            .replacingOccurrences(of: ":", with: "")
        var urls: [URL] = []
        for (index, item) in picked.enumerated() {
            let ext = item.supportedContentTypes.first?.preferredFilenameExtension ?? "jpg"
            let suffix = picked.count > 1 ? "-\(index + 1)" : ""
            do {
                guard let data = try await item.loadTransferable(type: Data.self) else { continue }
                let url = try AttachmentFiles.tempURL(name: "IMG-\(stamp)\(suffix).\(ext)")
                try data.write(to: url)
                urls.append(url)
            } catch {
                failure = error.localizedDescription
            }
        }
        if !urls.isEmpty { attach(urls) }
    }
}
