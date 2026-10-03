import SwiftUI

struct NowPlayingView: View {
    @EnvironmentObject private var store: WatchStore

    var body: some View {
        Group {
            if let media = store.snapshot?.media {
                TimelineView(.periodic(from: .now, by: 0.5)) { context in
                    player(media, position: store.position(of: media, at: context.date))
                }
            } else if store.snapshot == nil {
                ProgressView()
            } else {
                EmptyNote(symbol: "music.note", text: t("没有在播放", "Nothing playing"))
            }
        }
        .page(t("播放", "Now Playing"), tint: Palette.music)
    }

    private func player(_ media: Media, position: Double?) -> some View {
        VStack(spacing: 6) {
            HStack(spacing: 8) {
                artwork
                VStack(alignment: .leading, spacing: 1) {
                    Text(media.title.isEmpty ? media.appName : media.title)
                        .font(.headline)
                        .lineLimit(1)
                    Text(media.artist.isEmpty ? media.appName : media.artist)
                        .font(.footnote)
                        .foregroundStyle(.secondary)
                        .lineLimit(1)
                }
                Spacer(minLength: 0)
            }

            lyric(store.lyric(at: position))

            Spacer(minLength: 0)

            progress(media, position: position)

            HStack {
                control("backward.fill", action: "previous")
                control(media.playing ? "pause.fill" : "play.fill", action: "toggle", prominent: true)
                control("forward.fill", action: "next")
            }
        }
    }

    private var artwork: some View {
        Group {
            if let image = store.artwork {
                Image(uiImage: image)
                    .resizable()
                    .scaledToFill()
            } else {
                Image(systemName: "music.note")
                    .foregroundStyle(.secondary)
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
                    .background(.white.opacity(0.12))
            }
        }
        .frame(width: 40, height: 40)
        .clipShape(RoundedRectangle(cornerRadius: 8))
    }

    /// The line being sung, and its translation when there is one.
    @ViewBuilder
    private func lyric(_ line: LyricLine?) -> some View {
        if let line, !line.text.isEmpty {
            VStack(spacing: 1) {
                Text(line.text)
                    .font(.footnote.weight(.medium))
                    .lineLimit(2)
                if let translation = line.translation, !translation.isEmpty {
                    Text(translation)
                        .font(.caption2)
                        .foregroundStyle(.secondary)
                        .lineLimit(1)
                }
            }
            .multilineTextAlignment(.center)
            .frame(maxWidth: .infinity)
            .id(line.at)
            .transition(.opacity)
            .animation(.easeInOut(duration: 0.25), value: line.at)
        }
    }

    @ViewBuilder
    private func progress(_ media: Media, position: Double?) -> some View {
        if let position, let duration = media.duration, duration > 0 {
            VStack(spacing: 2) {
                ProgressView(value: min(position / duration, 1))
                    .tint(Palette.music)
                HStack {
                    Text(clock(position))
                    Spacer()
                    Text("-" + clock(duration - position))
                }
                .font(.caption2.monospacedDigit())
                .foregroundStyle(.secondary)
            }
        }
    }

    private func control(_ symbol: String, action: String, prominent: Bool = false) -> some View {
        Button {
            store.media(action)
        } label: {
            Image(systemName: symbol)
                .font(prominent ? .title3 : .body)
                .frame(maxWidth: .infinity, minHeight: prominent ? 40 : 34)
        }
        .buttonStyle(.bordered)
        .buttonBorderShape(.circle)
        .tint(prominent ? Palette.music : nil)
    }
}
