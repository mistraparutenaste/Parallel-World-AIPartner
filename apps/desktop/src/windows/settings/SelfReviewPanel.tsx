import type { SelfReviewDto } from '@parallel-world/contracts';
import { invoke } from '@tauri-apps/api/core';
import { useEffect, useState } from 'react';

export function SelfReviewPanel() {
  const [review, setReview] = useState<SelfReviewDto | null>(null);
  const [loading, setLoading] = useState(true);
  const [refreshing, setRefreshing] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    void invoke<SelfReviewDto | null>('get_self_review')
      .then(setReview)
      .catch(() => setError('振り返りを読み込めませんでした。'))
      .finally(() => setLoading(false));
  }, []);

  const regenerate = async () => {
    setRefreshing(true);
    setError(null);
    try {
      setReview(await invoke<SelfReviewDto | null>('regenerate_self_review'));
    } catch {
      setError('振り返りを更新できませんでした。LLM設定を確認してください。');
    } finally {
      setRefreshing(false);
    }
  };

  return (
    <section aria-labelledby="self-review-heading">
      <h2 id="self-review-heading">あなたについて</h2>
      <p>会話を人間向けに振り返った表示です。AIの会話プロンプトには使用しません。</p>
      {loading ? <p>読み込み中…</p> : null}
      {!loading && !review ? <p>振り返る会話がまだありません。</p> : null}
      {review ? <p className="self-review-content">{review.content}</p> : null}
      {error ? <p role="alert">{error}</p> : null}
      <button type="button" disabled={refreshing} onClick={() => void regenerate()}>
        {refreshing ? '更新中…' : '振り返りを更新'}
      </button>
    </section>
  );
}
