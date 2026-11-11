// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
import AppKit
import Foundation

guard CommandLine.arguments.count == 2,
      let data = try? Data(contentsOf: URL(fileURLWithPath: CommandLine.arguments[1])),
      let image = NSBitmapImageRep(data: data),
      image.pixelsWide >= 4, image.pixelsHigh >= 4 else {
    fputs("Cannot read probe PNG\n", stderr)
    exit(1)
}

func hexAt(_ x: Int) -> String? {
    guard let color = image.colorAt(x: x, y: image.pixelsHigh / 2)?.usingColorSpace(.deviceRGB) else {
        return nil
    }
    let red = Int((color.redComponent * 255).rounded())
    let green = Int((color.greenComponent * 255).rounded())
    let blue = Int((color.blueComponent * 255).rounded())
    return String(format: "%02X%02X%02X", red, green, blue)
}

guard let cookie = hexAt(image.pixelsWide / 4),
      let storage = hexAt(image.pixelsWide * 3 / 4) else {
    fputs("Cannot sample probe PNG\n", stderr)
    exit(1)
}
print("\(cookie) \(storage)")
