// The item panel's attachments (spec/attachments.md): the files on the
// item, under its subtasks. A column of rows, each a file's glyph, name
// and size, over the uploads still on their way and a trailing "Upload
// file" row that opens the system's file picker. The web's
// `FilesField.tsx`.
//
// The bytes are online-only: a click has the engine fetch and decrypt
// the file into the Downloads folder, as the web downloads it, and
// Finder shows it there. It is never opened from here: the name and
// type are whatever a member of the workspace wrote. Removing a file
// is permanent, so it asks first; the cross under the pointer and the
// row's menu both offer it. A binned item's files are shown and saved,
// and nothing is added or removed.

import AppKit
import MonoplanCore
import SwiftUI
import UniformTypeIdentifiers

struct FilesField: View {
    let files: [ItemFile]
    /// Attaches under way or failed, for this item.
    let uploads: [FileUpload]
    /// A binned item's: shown, not edited.
    var locked = false
    /// There is an account to hold the bytes.
    var canAttach = true
    /// Files were picked for the item.
    let attach: ([URL]) -> Void
    /// Fetch a file into Downloads; where it landed arrives on the
    /// main queue.
    let open: (ItemFile, @escaping (Result<URL, Error>) -> Void) -> Void
    let remove: (ItemFile) -> Void
    let dismissUpload: (UUID) -> Void

    /// One line's row, as a subtask's.
    static let rowHeight: CGFloat = 28

    @State private var importing = false
    @State private var attachHovered = false
    /// The file being fetched, by id.
    @State private var opening: String?
    @State private var removing: ItemFile?
    /// The last open that failed: the file's id and why.
    @State private var failure: (id: String, message: String)?

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            ForEach(files) { file in
                FileRow(
                    file: file, locked: locked,
                    opening: opening == file.id,
                    failure: failure?.id == file.id ? failure?.message : nil,
                    open: { show(file) },
                    remove: { removing = file }
                )
            }
            ForEach(uploads) { upload in
                UploadRow(upload: upload) { dismissUpload(upload.id) }
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
    }

    private var attachRow: some View {
        // Filled under the pointer, as the panel's date and place rows
        // (`panelBox`): the fill reaches past the row on each side, so
        // the glyph stays in the files' leading column.
        let shape = RoundedRectangle(cornerRadius: PanelBox.radius, style: .continuous)
        return Button {
            importing = true
        } label: {
            HStack(alignment: .center, spacing: 5) {
                PanelGlyph(image: NavArt.upload)
                    .frame(width: 20)
                Text("Upload file")
                    .lineLimit(1)
                Spacer(minLength: 0)
            }
            .foregroundStyle(.tertiary)
            .frame(minHeight: Self.rowHeight)
            .padding(.horizontal, PanelBox.inset)
            .background(.quaternary.opacity(attachHovered ? 0.5 : 0), in: shape)
            .contentShape(shape)
        }
        .buttonStyle(.plain)
        .onHover { attachHovered = $0 }
        .padding(.horizontal, -PanelBox.inset)
    }

    private func show(_ file: ItemFile) {
        guard opening == nil else { return }
        opening = file.id
        failure = nil
        open(file) { result in
            opening = nil
            switch result {
            case .success(let url): NSWorkspace.shared.activateFileViewerSelecting([url])
            case .failure(let error): failure = (file.id, engineErrorMessage(error))
            }
        }
    }
}

/// One attachment: its glyph, its name, and its size at the trailing
/// edge, where the fetch's spinner or its failure stands in for it. The
/// cross that removes it shows under the pointer.
private struct FileRow: View {
    let file: ItemFile
    let locked: Bool
    let opening: Bool
    let failure: String?
    let open: () -> Void
    let remove: () -> Void
    @State private var hovered = false

    var body: some View {
        HStack(alignment: .center, spacing: 5) {
            Image(systemName: AttachmentFiles.symbol(name: file.name))
                .foregroundStyle(.secondary)
                .frame(width: 20)
            Text(verbatim: file.name)
                .lineLimit(1)
                .truncationMode(.middle)
            Spacer(minLength: 8)
            if opening {
                ProgressView().controlSize(.small)
            } else if let failure {
                Text(verbatim: failure)
                    .font(.footnote)
                    .foregroundStyle(.red)
                    .lineLimit(1)
            } else {
                Text(verbatim: AttachmentFiles.sizeText(file.size))
                    .font(.footnote)
                    .foregroundStyle(.secondary)
                    .monospacedDigit()
            }
            if hovered, !locked {
                Button(action: remove) {
                    Image(systemName: "xmark")
                        .font(.footnote.weight(.semibold))
                        .foregroundStyle(.secondary)
                        .frame(width: 16, height: 16)
                        .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .help("Delete file")
                .accessibilityLabel(Text("Delete file"))
            }
        }
        .frame(minHeight: FilesField.rowHeight)
        .contentShape(Rectangle())
        .onTapGesture(perform: open)
        .onHover { hovered = $0 }
        .contextMenu {
            Button("Save to Downloads", action: open)
            if !locked {
                Button("Delete…", role: .destructive, action: remove)
            }
        }
        .accessibilityElement(children: .combine)
        .accessibilityAddTraits(.isButton)
    }
}

/// An attach on its way: a spinner and the file's name; one that
/// failed: why, and a cross to dismiss it.
private struct UploadRow: View {
    let upload: FileUpload
    let dismiss: () -> Void

    var body: some View {
        HStack(alignment: .center, spacing: 5) {
            Group {
                if upload.error == nil {
                    ProgressView().controlSize(.small)
                } else {
                    Image(systemName: "exclamationmark.triangle").foregroundStyle(.red)
                }
            }
            .frame(width: 20)
            Text(verbatim: upload.name)
                .lineLimit(1)
                .truncationMode(.middle)
                .foregroundStyle(.secondary)
            Spacer(minLength: 8)
            if let error = upload.error {
                Text(verbatim: error)
                    .font(.footnote)
                    .foregroundStyle(.red)
                    .lineLimit(1)
                    .help(Text(verbatim: error))
                Button(action: dismiss) {
                    Image(systemName: "xmark")
                        .font(.footnote.weight(.semibold))
                        .foregroundStyle(.secondary)
                        .frame(width: 16, height: 16)
                        .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .help("Dismiss")
                .accessibilityLabel(Text("Dismiss"))
            }
        }
        .frame(minHeight: FilesField.rowHeight)
    }
}
