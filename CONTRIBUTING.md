# コントリビューションについて

## ライセンスと権利の扱い

Parallel Worldはデュアルライセンスです。[LICENSE](LICENSE)（PolyForm Noncommercial License 1.0.0）で非商用利用を許諾し、商用利用は著作権者との個別契約とします（[LICENSE-COMMERCIAL.md](LICENSE-COMMERCIAL.md)）。

この形態を維持するため、**Pull Requestを送信した時点で、あなたは次の各号に同意したものとみなします。**

1. あなたが提出する内容（以下「本コントリビューション」）は、あなた自身が作成したものであり、あなたがこの許諾を与える権利を有していること。
2. 本コントリビューションについて、著作権者に対し、期間・地域の制限のない、取消不能、無償、譲渡可能、再許諾可能な権利を許諾すること。この許諾には、複製、改変、二次的著作物の作成、頒布、公衆送信、および**本プロジェクトの商用ライセンスを含む任意のライセンス条件での再許諾**が含まれます。
3. 本コントリビューションに関してあなたが有する特許について、同様の許諾を与えること。
4. 本コントリビューションが第三者の権利を侵害しないこと。第三者のコードやアセットを含む場合は、その出所とライセンスをPull Requestに明記すること。

著作権はあなたに残りますが、上記の許諾により著作権者は本プロジェクトを商用ライセンスで提供し続けられます。この条件に同意できない場合は、Pull Requestではなくissueでの提案をご利用ください。

雇用主の権利が及ぶ立場でコントリビューションを行う場合は、事前に雇用主の許可を得てください。

## 開発の進め方

品質ゲートは[README](README.md)に記載のコマンドで確認します。Pull Requestを出す前に、次がすべて通ることを確認してください。

```bash
corepack pnpm typecheck && corepack pnpm test && cargo clippy --workspace --all-targets --all-features -- -D warnings && cargo test --workspace
```

第三者のコード、モデル、フォント、キャラクターアセットを新たに追加する場合は、[THIRD-PARTY-NOTICES.md](THIRD-PARTY-NOTICES.md)への追記も同じPull Requestに含めてください。コピーレフト系（GPL / AGPL / LGPL）の依存はPolyForm Noncommercial Licenseと矛盾するため受け入れられません。
