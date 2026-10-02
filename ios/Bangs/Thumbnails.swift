import ImageIO
import SwiftUI
import UIKit

/// Small pictures of downloaded files for the lists, made off the main thread and kept.
enum Thumbnails {
    private static let cache = NSCache<NSURL, UIImage>()

    static func load(_ url: URL, side: CGFloat) async -> UIImage? {
        if let hit = cache.object(forKey: url as NSURL) {
            return hit
        }
        let made = await Task.detached(priority: .utility) { () -> UIImage? in
            guard let source = CGImageSourceCreateWithURL(url as CFURL, nil) else { return nil }
            let options: [CFString: Any] = [
                kCGImageSourceCreateThumbnailFromImageAlways: true,
                kCGImageSourceCreateThumbnailWithTransform: true,
                kCGImageSourceThumbnailMaxPixelSize: side,
            ]
            guard let image = CGImageSourceCreateThumbnailAtIndex(source, 0, options as CFDictionary) else { return nil }
            return UIImage(cgImage: image)
        }.value
        if let made = made {
            cache.setObject(made, forKey: url as NSURL)
        }
        return made
    }
}

/// A square thumbnail of a downloaded picture.
struct ThumbnailView: View {
    let url: URL
    var side: CGFloat = 44

    @State private var image: UIImage?

    var body: some View {
        ZStack {
            Color(UIColor.tertiarySystemFill)
            if let image = image {
                Image(uiImage: image)
                    .resizable()
                    .scaledToFill()
            }
        }
        .frame(width: side, height: side)
        .clipShape(RoundedRectangle(cornerRadius: 8, style: .continuous))
        .task(id: url) {
            image = await Thumbnails.load(url, side: side * 3)
        }
    }
}
