# Vendored Live2D SDK components

このディレクトリの内容は手編集しない。更新時は下記の出所から再コピーする。

## framework/

- 出所: [Live2D/CubismWebFramework](https://github.com/Live2D/CubismWebFramework)
- バージョン: 5-r.5（コミット d4da0aa07e47d2c1e4f5fa7ea6047861ea5e5d0b、`third_party/live2d/CubismWebSamples/Framework` サブモジュールから複製）
- ライセンス: Live2D Open Software License Agreement（`framework/LICENSE.md`）

## core/

- 出所: Cubism SDK for Web 5-r.5（公式サイト配布物、`project-input/live2d/SOURCE_URLS.md` 参照）
- ライセンス: Live2D Proprietary Software License Agreement（`core/LICENSE.md`）
- ここに置くのは `core/RedistributableFiles.txt` に列挙された再配布許可ファイルのみ（型定義 `live2dcubismcore.d.ts`）。実行体 `live2dcubismcore.min.js` は `apps/desktop/public/live2d/core/` に配置。

WebGLシェーダー（Framework付属）は `apps/desktop/public/live2d/shaders/` に配置し、実行時にfetchされる。
