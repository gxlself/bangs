import SwiftUI
import WatchKit

struct NowPlayingView: View {
    @EnvironmentObject private var store: WatchStore

    var body: some View {
        Group {
            if let media = store.snapshot?.media {
                TimelineView(.periodic(from: .now, by: 0.5)) { context in
                    player(media, position: store.position(of: media, at: context.date))
                        // The page keeps a strip clear above the screen's
                        // rounded bottom, 19 points on a 40 mm watch and 40 on
                        // an Ultra. The player goes partway into it: lower
                        // controls on the big watches, room for a lyric line on
                        // the small ones, and the side buttons still clear of
                        // the corners. Not while the offline note is there.
                        .padding(.bottom, reclaimsBottom ? Self.bottomMargin : 0)
                        .ignoresSafeArea(.container, edges: reclaimsBottom ? .bottom : [])
                }
            } else if store.snapshot == nil {
                ProgressView()
            } else {
                EmptyNote(symbol: "music.note", text: t("没有在播放", "Nothing playing"))
            }
        }
        // No title: the track's own name heads the page, and the screen is
        // small enough without a second one above it.
        .page(nil, tint: Palette.music)
    }

    private var reclaimsBottom: Bool { store.connection != .offline }

    /// From the bottom of the screen to the bottom of the controls.
    private static let bottomMargin = (WKInterfaceDevice.current().screenBounds.height * 0.075).rounded()

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

            // The room between the title and the controls, whatever the watch:
            // the lyric shows as much of itself as fits there.
            lyric(store.lyric(at: position))
                .frame(maxHeight: .infinity)

            progress(media, position: position)

            HStack(spacing: 8) {
                control("backward.fill", action: "previous")
                control(media.playing ? "pause.fill" : "play.fill", action: "toggle", prominent: true)
                control("forward.fill", action: "next")
            }
            // In from the edges, where the screen's corners round off.
            .padding(.horizontal, 4)
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
        .frame(width: 36, height: 36)
        .clipShape(RoundedRectangle(cornerRadius: 8))
    }

    /// The line being sung, and its translation when there is one — as much
    /// of that as fits above the controls: the line wrapped, the line on its
    /// own, or nothing on a small watch with a long line.
    @ViewBuilder
    private func lyric(_ line: LyricLine?) -> some View {
        if let line, !line.text.isEmpty {
            let translation = line.translation.flatMap { $0.isEmpty ? nil : $0 }
            ViewThatFits(in: .vertical) {
                lyricLines(line.text, translation, lines: 2)
                lyricLines(line.text, translation, lines: 1)
                lyricLines(line.text, nil, lines: 1)
                Color.clear.frame(height: 0)
            }
            .id(line.at)
            .transition(.opacity)
            .animation(.easeInOut(duration: 0.25), value: line.at)
        }
    }

    private func lyricLines(_ text: String, _ translation: String?, lines: Int) -> some View {
        VStack(spacing: 1) {
            Text(text)
                .font(.footnote.weight(.medium))
                .lineLimit(lines)
            if let translation {
                Text(translation)
                    .font(.caption2)
                    .foregroundStyle(.secondary)
                    .lineLimit(lines)
            }
        }
        .multilineTextAlignment(.center)
        .frame(maxWidth: .infinity)
        .fixedSize(horizontal: false, vertical: true)
    }

    @ViewBuilder
    private func progress(_ media: Media, position: Double?) -> some View {
        if let position, let duration = media.duration, duration > 0 {
            // One row: the times either side of a thin bar.
            HStack(spacing: 6) {
                Text(clock(position))
                GeometryReader { geometry in
                    ZStack(alignment: .leading) {
                        Capsule().fill(.white.opacity(0.18))
                        Capsule()
                            .fill(Palette.music)
                            .frame(width: geometry.size.width * min(max(position / duration, 0), 1))
                    }
                }
                .frame(height: 4)
                Text("-" + clock(duration - position))
            }
            .font(.caption2.monospacedDigit())
            .foregroundStyle(.secondary)
        }
    }

    private func control(_ symbol: String, action: String, prominent: Bool = false) -> some View {
        Button {
            store.media(action)
        } label: {
            Image(systemName: symbol)
                .font(prominent ? .title3 : .body)
                .frame(maxWidth: .infinity, minHeight: prominent ? 36 : 32)
        }
        .buttonStyle(.bordered)
        .buttonBorderShape(.circle)
        .tint(prominent ? Palette.music : nil)
    }
}
