import { invoke } from '@tauri-apps/api/core';
import { useState } from 'react';
export function DataPanel() {
 const [destination,setDestination]=useState(''); const [status,setStatus]=useState<string|null>(null);
 const run=(command:string,args?:Record<string,string>)=>(args===undefined?invoke(command):invoke(command,args)).then(()=>setStatus('完了しました')).catch(e=>setStatus(String(e)));
 return <section aria-labelledby="data-title"><h2 id="data-title">データ</h2><label htmlFor="export-path">保存先</label><input id="export-path" value={destination} onChange={e=>setDestination(e.target.value)}/><button type="button" disabled={!destination.trim()} onClick={()=>run('export_user_data',{destination:destination.trim()})}>エクスポート</button><button type="button" onClick={()=>{if(window.confirm('会話履歴を削除しますか？'))void run('delete_conversation_history')}}>会話履歴を削除</button><button type="button" onClick={()=>{if(window.confirm('要約と長期記憶を削除しますか？'))void run('delete_memories')}}>記憶を削除</button>{status&&<p role="status">{status}</p>}</section>;
}
