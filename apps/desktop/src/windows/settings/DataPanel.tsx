import { invoke } from '@tauri-apps/api/core';
import { useState } from 'react';
export function DataPanel() {
 const [destination,setDestination]=useState(''); const [status,setStatus]=useState<string|null>(null); const [error,setError]=useState<string|null>(null); const [busy,setBusy]=useState(false);
 const run=async(command:string,args?:Record<string,unknown>)=>{setBusy(true);setError(null);try{await(args===undefined?invoke(command):invoke(command,args));setStatus('完了しました')}catch(e){setError(String(e))}finally{setBusy(false)}};
 const exportData=()=>{const overwrite=window.confirm('保存先に既存ファイルがある場合、上書きしますか？');void run('export_user_data',{destination:destination.trim(),allowOverwrite:overwrite})};
 return <section aria-labelledby="data-title"><h2 id="data-title">データ</h2><p>会話履歴、要約、長期記憶を管理します。</p><label htmlFor="export-path">保存先</label><input id="export-path" disabled={busy} value={destination} onChange={e=>setDestination(e.target.value)}/><button type="button" disabled={busy||!destination.trim()} onClick={exportData}>エクスポート</button><button type="button" disabled={busy} onClick={()=>{if(window.confirm('会話履歴のみ削除します。要約と長期記憶は残ります。続行しますか？'))void run('delete_conversation_history')}}>会話履歴を削除</button><button type="button" disabled={busy} onClick={()=>{if(window.confirm('要約と長期記憶のみ削除します。会話履歴は残ります。続行しますか？'))void run('delete_memories')}}>記憶を削除</button>{status&&<p role="status">{status}</p>}{error&&<p role="alert">{error}</p>}</section>;
}
