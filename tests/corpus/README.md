# Corpus

Asset catalogs compiled by Apple's actool, from early asset-catalog builds (CoreUI 308) to Xcode
27.1, taken from open-source projects at the commit or release listed, with the inputs they were
compiled from where those are public. Each file stays under its project's licence; the icons are
those projects' marks and are here only to test accord against Apple's output.

The set is kept small and chosen for coverage: each sample brings platforms, catalog features,
input options or differences from accord's output that no other sample has. A new sample should
add something the others lack, and its project's licence must allow redistribution.

Binaries are in Git LFS: run `git lfs pull` after cloning. `corpus.toml` has each file's exact
origin and the compile arguments of the pairs.

| Sample | Platform | Project | Commit / release | Licence | Compile pair |
|---|---|---|---|---|---|
| `brew-browser` | macOS | [Brew Browser](https://github.com/msitarzewski/brew-browser) | `ed2eea7eea08` | MIT | yes |
| `readinglist` | macOS | [Reading List](https://github.com/marckohlbrugge/readinglist) | `89ba77273480` | MIT | yes |
| `mediainfo` | macOS | [MediaInfo](https://github.com/MediaArea/MediaInfo) | `61082c82b555` | BSD-2-Clause | yes |
| `moddable` | macOS | [Moddable SDK (mcsim)](https://github.com/Moddable-OpenSource/moddable) | `80076c48f5ab` | GPL-3.0 | yes |
| `aven` | macOS | [Aven](https://github.com/raine/aven) | `35d55d8cfcbc` | MIT | yes |
| `zettlr` | macOS | [Zettlr](https://github.com/Zettlr/Zettlr) | `c805b1e3ec8c` | GPL-3.0 | yes |
| `emacs-liquid-glass` | macOS | [Emacs Liquid Glass Icons](https://github.com/jimeh/emacs-liquid-glass-icons) | `6e25183fe5a4` | GPL-3.0 | yes |
| `tusk` | macOS | [Tusk](https://github.com/alpcanaydin/tusk) | `5285238fb750` | MIT | yes |
| `browseros` | macOS | [BrowserOS](https://github.com/browseros-ai/BrowserOS) | `7fdd50f80800` | AGPL-3.0 | yes |
| `daintree` | macOS | [Daintree](https://github.com/daintreehq/daintree) | `f45fa04b3b61` | Apache-2.0 | yes |
| `sentinel` | macOS | [Sentinel](https://github.com/RemakeCode/sentinel) | `0cbf6eafcf3f` | MIT | yes |
| `kdrive` | macOS | [kDrive](https://github.com/Infomaniak/desktop-kDrive) | `643057e04581` | GPL-3.0 |  |
| `sparkle-test-app` | macOS | [Sparkle 1.26.0 Test App (vendored by Oni2)](https://github.com/onivim/oni2) | `161a92c22267` | MIT |  |
| `ios/roothide` | iOS | [RootHide Dopamine](https://github.com/P013onEr/RootHide) | `c7f6b12929c4` (`AppIcon.appiconset`: `92bb51800786`) | MIT | yes |
| `ios/openutau` | iOS | [OpenUtau Mobile](https://github.com/vocoder712/OpenUtauMobile) | `17a86f602e05` | Apache-2.0 | yes |
| `ios/swiftgen` | iOS | [SwiftGen](https://github.com/SwiftGen/SwiftGen) | `ddb03a73cab2` | MIT |  |
| `ios/sumup` | iOS | [SumUp iOS SDK](https://github.com/sumup/sumup-ios-sdk) | `8224b6eb435d` | Apache-2.0 |  |
| `tvos/scummvm` | tvOS | [ScummVM](https://github.com/scummvm/scummvm) | `fd1b51d98dbe` | GPL-3.0 |  |
| `maccatalyst/swiftyvk` | Mac Catalyst | [SwiftyVK](https://github.com/SwiftyVK/SwiftyVK) | `47f72fc766f9` | MIT |  |
| `legacy/pxcore2` | legacy | [pxCore](https://github.com/pxscene/pxCore2) | `a33afddbf01c` | Apache-2.0 |  |
| `legacy/acextract` | legacy | [acextract](https://github.com/bartoszj/acextract) | `df3b018d53cd` | MIT |  |
| `vlc` | macOS | [VLC](https://code.videolan.org/videolan/vlc) | `665f5f6168cd` | GPL-2.0 | yes |
| `ios/utm-remote` | iOS | [UTM Remote 5.0.8](https://github.com/utmapp/UTM) | `v5.0.8` | Apache-2.0 | yes |
