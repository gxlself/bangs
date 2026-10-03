import SwiftUI

/// What the Mac is playing: artwork, the line being sung and the transport.
struct NowPlayingView: View {
    @EnvironmentObject private var store: WatchStore

    var body: some View {
        NavigationStack {
            Connected { state in
                if let media = state.media {
                    ScrollView {
                        player(media, lyrics: state.lyrics)
                    }
                } else {
                    EmptyState(symbol: "music.note", text: "Mac 上没有在播放")
                }
            }
            .navigationTitle("正在播放")
        }
    }

    private func player(_ media: Media, lyrics: [LyricLine]) -> some View {
        VStack(spacing: 8) {
            HStack(spacing: 8) {
                artwork
                    .frame(width: 44, height: 44)
                    .clipShape(RoundedRectangle(cornerRadius: 8))
                VStack(alignment: .leading, spacing: 1) {
                    Text(media.title)
                        .font(.headline)
                        .lineLimit(1)
                    Text(media.artist.isEmpty ? media.appName : media.artist)
                        .font(.footnote)
                        .foregroundStyle(.secondary)
                        .lineLimit(1)
                }
                Spacer(minLength: 0)
            }

            TimelineView(.periodic(from: .now, by: 0.5)) { context in
                let position = media.position(at: context.date, clockOffset: store.clockOffset)
                VStack(spacing: 6) {
                    if let position, let line = lyrics.line(at: position), !line.text.isEmpty {
                        VStack(spacing: 1) {
                            Text(line.text)
                                .font(.system(.body, design: .rounded))
                                .multilineTextAlignment(.center)
                                .lineLimit(2)
                            if let translation = line.translation, !translation.isEmpty {
                                Text(translation)
                                    .font(.caption2)
                                    .foregroundStyle(.secondary)
                                    .lineLimit(1)
                            }
                        }
                        .frame(maxWidth: .infinity)
                    }
                    if let position, let duration = media.duration, duration > 0 {
                        ProgressView(value: position, total: duration)
                            .tint(.accentColor)
                    }
                }
            }

            HStack(spacing: 4) {
                transport("backward.fill", action: "previous")
                transport(media.playing ? "pause.fill" : "play.fill", action: "toggle", prominent: true)
                transport("forward.fill", action: "next")
            }
        }
    }

    @ViewBuilder private var artwork: some View {
        if let image = store.artwork {
            Image(uiImage: image)
                .resizable()
                .scaledToFill()
        } else {
            ZStack {
                Color.gray.opacity(0.3)
                Image(systemName: "music.note")
                    .foregroundStyle(.secondary)
            }
        }
    }

    private func transport(_ symbol: String, action: String, prominent: Bool = false) -> some View {
        Button {
            store.media(action)
        } label: {
            Image(systemName: symbol)
                .font(prominent ? .title2 : .body)
                .frame(maxWidth: .infinity, minHeight: 40)
                .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
    }
}
