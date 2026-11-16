"""Maintainer helper: regenerate schema.json, no third-party dependencies."""
import json
from pathlib import Path

def obj(props, required=None):
    return {'type':'object','properties':props,'required':list(props) if required is None else required}
def ref(n): return {'$ref':'#/$defs/'+n}
def enum(*v): return {'enum':list(v)}
s={'type':'string'}; ident={'type':'string','minLength':1}; boolean={'type':'boolean'}
nullable={'type':['string','null']}
revision={'type':'integer','minimum':1,'maximum':9007199254740991}
offset={'type':'integer','minimum':0,'maximum':9007199254740991}
count=offset; positive=revision
def array(items, most=None, **extra):
    a={'type':'array','items':items,**extra}
    if most is not None: a['maxItems']=most
    return a
names={'type':'array','uniqueItems':True,'items':ident}
doc_revision={'type':'string','pattern':'^([0-9a-f]{16}|absent)$'}
def nullref(n): return {'anyOf':[ref(n),{'type':'null'}]}
d={}
d['policy']=obj({'plan':enum('allow','forbid'),'approval':enum('default','reject')})
d['turn']=obj({'id':ident,'state':enum('running','finished','interrupted','failed'),'error':nullable})
d['message']=obj({'role':enum('system','user','assistant','tool'),'content':s})
d['pending_approval']=obj({'call_id':ident,'reason':nullable})
d['gate']=obj({'kind':enum('tool','plan','mission','unknown'),'reason':nullable,'supported':boolean})
d['state']=obj({'session_id':ident,'revision':revision,'external_approvals':boolean,'pending_approval':nullref('pending_approval'),'gate':nullref('gate'),'cwd':ident,'model_id':s,'policy':ref('policy'),'turn':nullref('turn'),'working':boolean,'awaiting_approval':boolean,'approval_reason':nullable,'pending_count':{'type':'integer','minimum':0,'maximum':5},'messages':{'type':'array','items':ref('message')},'streaming':nullable})
d['attachment']={'oneOf':[obj({'kind':{'const':'image_file'},'path':ident}),obj({'kind':{'const':'pasted_text'},'text':s,'name':nullable},['kind','text'])]}
input_props={'text':{'type':'string','minLength':1},'attachments':{'type':'array','maxItems':16,'items':ref('attachment')}}
roles=('main','awareness','safeguard','compactor','planner')
provider_in=obj({'name':ident,'endpoint':ident,'api_type':enum('openai','anthropic'),'api_key':{'type':['string','null'],'minLength':1}},['name','endpoint','api_type'])
assignment=obj({'provider':ident,'model_id':ident,'route':ident},['provider','model_id'])
session_name={'type':'string','minLength':1,'maxLength':200}
v1_params={'create':obj({'cwd':ident,'model_id':ident,'policy':ref('policy'),'external_approvals':boolean,'name':session_name},['cwd']),'resume':obj({'session_id':ident,'policy':ref('policy'),'external_approvals':boolean},['session_id']),'state':obj({}),'submit':obj(input_props,['text']),'enqueue':obj(input_props,['text']),'interrupt':obj({'turn_id':ident}),'interrupt_and_submit':obj({'turn_id':ident,**input_props},['turn_id','text']),'approve':obj({'call_id':ident,'approve':boolean}),'configure':obj({'providers':{'type':'array','maxItems':32,'items':provider_in},'models':obj({role:assignment for role in roles},[])},[]),'detach':obj({})}
public_role=enum(*roles)
effort={'type':'string','minLength':1,'maxLength':64,'pattern':'^[^\\s\\x00-\\x1f\\x7f]+$'}
limit_params={'max_output_tokens':{'anyOf':[{'const':0},{'type':'integer','minimum':256,'maximum':1000000}]},'context_window_limit':{'anyOf':[{'const':0},{'type':'integer','minimum':1024,'maximum':100000000}]},'context_model_alias':{'type':'string','maxLength':200}}
host_params={'config.get':obj({'view':enum('global','session')},[]),'config.providers.delete':obj({'provider_uuid':ident,'if_revision':doc_revision,'dry_run':boolean},['provider_uuid']),'config.models.delete':obj({'model_uuid':ident,'if_revision':doc_revision,'dry_run':boolean},['model_uuid']),'config.roles.unassign':obj({'role':public_role,'scope':enum('global','session'),'if_revision':doc_revision},['role']),'config.effort.get':obj({'wait_ms':{'type':'integer','minimum':0,'maximum':10000}},[]),'config.effort.set':obj({'effort':effort,'if_main_model':nullable},['effort']),'config.limits.get':obj({}),'config.limits.set':{**obj(limit_params,[]),'anyOf':[{'required':[k]} for k in limit_params]},'sessions.info':obj({})}
cursor={'type':'string','minLength':1}
skill_name={'type':'string','minLength':1,'maxLength':64,'pattern':'^[a-z0-9]+(-[a-z0-9]+)*$'}
skill_ref={'type':'string','minLength':1}
tiny=lambda n,most:array({'type':'string','minLength':1,'maxLength':n},most)
doc_text={'type':'string','minLength':1,'maxLength':2097152}
session_params={
 'sessions.list':obj({'kind':enum('coding','room','all'),'live':boolean,'limit':{'type':'integer','minimum':1,'maximum':500},'cursor':cursor},[]),
 'sessions.rename':obj({'session_id':ident,'name':session_name}),
 'sessions.delete':obj({'session_id':ident}),
 'sessions.stop':obj({'session_id':ident}),
 'sessions.transcript':obj({'session_id':ident,'since':offset,'limit':{'type':'integer','minimum':1,'maximum':500}},['session_id']),
}
usage_params={'usage.query':obj({'from_at':{'type':'integer'},'to_at':{'type':'integer'},'session_id':ident,'workdir':ident,'group_by':enum('none','model','role','session'),'bucket':enum('none','hour','day','week'),'tz_offset_minutes':{'type':'integer','minimum':-720,'maximum':840},'limit':{'type':'integer','minimum':1,'maximum':500}},[])}
skill_address={'tier':skill_ref,'workdir':ident,'name':ident}
skill_params={
 'skills.list':obj({'scope':enum('computer','project','external','all'),'tier':skill_ref,'workdir':ident,'include_diagnostics':boolean,'limit':{'type':'integer','minimum':1,'maximum':1000},'cursor':cursor},[]),
 'skills.get':obj({**skill_address,'include_raw':boolean},['name']),
 'skills.read_file':obj({**skill_address,'path':ident,'generation':ident},['name','path']),
 'skills.create':obj({'scope':enum('computer','project'),'workdir':ident,'name':skill_name,'description':{'type':'string','minLength':1,'maxLength':2000},'instruction':doc_text,'triggers':tiny(200,32),'allowed_tools':tiny(100,64)},['scope','name','description','instruction']),
 'skills.update':obj({'tier':skill_ref,'workdir':ident,'name':ident,'generation':ident,'patch':{**obj({'name':skill_name,'description':{'type':'string','minLength':1,'maxLength':2000},'triggers':tiny(200,32),'allowed_tools':tiny(100,64),'instruction':doc_text},[]),'minProperties':1}},['tier','name','patch']),
 'skills.install':{**obj({'scope':enum('computer','project'),'workdir':ident,'name':skill_name,'data_b64':{'type':'string','minLength':1,'maxLength':39146840},'path':ident},['scope'])},
 'skills.remove':obj({'tier':skill_ref,'workdir':ident,'name':ident,'generation':ident},['tier','name']),
}
host_params.update(session_params); host_params.update(usage_params); host_params.update(skill_params)
# Room methods. Line numbers (line_seq) are per epoch and never the envelope seq.
line_seq={'type':'integer','minimum':1,'maximum':9007199254740991}
target=enum('template','room')
avatar_id={'type':'string','minLength':1,'maxLength':48,'pattern':'^[a-z0-9][a-z0-9_-]{0,47}$'}
tail={'type':'integer','minimum':0,'maximum':200}
goal_param={'type':'string','minLength':1}
room_params={
 'room.list':obj({'parent_session_id':ident,'live':boolean,'limit':{'type':'integer','minimum':1,'maximum':500},'cursor':cursor},[]),
 'room.open':obj({'parent_session_id':ident,'tail':tail,'wait_ms':{'type':'integer','minimum':0,'maximum':30000}},['parent_session_id']),
 'room.attach':obj({'session_id':ident,'tail':tail,'wait_ms':{'type':'integer','minimum':0,'maximum':15000}},['session_id']),
 'room.close':obj({'stop':boolean},[]),
 'room.send':obj({'text':{'type':'string','minLength':1},'to':ident,'shell':boolean,'attachments':{'type':'array','maxItems':16,'items':ref('attachment')}},['text']),
 'room.interrupt':obj({}),
 'room.control':obj({'action':enum('pause','resume','stop','redirect'),'avatar':ident,'note':{'type':'string','minLength':1,'maxLength':8192}},['action','avatar']),
 'room.status':obj({}),
 'room.history':{**obj({'epoch':ident,'after_line_seq':offset,'before_line_seq':line_seq,'limit':{'type':'integer','minimum':1,'maximum':200},'kinds':{'type':'array','items':ident},'author':ident},['epoch']),'not':{'required':['after_line_seq','before_line_seq']}},
 'room.shelf.list':obj({'prefix':{'type':'string','maxLength':200},'limit':{'type':'integer','minimum':1,'maximum':500},'cursor':cursor},[]),
 'room.item':obj({'key':{'type':'string','minLength':1,'maxLength':512}}),
 'room.trajectory':obj({'goal':goal_param},[]),
 'room.reports':obj({'include_markdown':boolean,'limit':{'type':'integer','minimum':1,'maximum':100},'cursor':cursor},[]),
 'room.spend':obj({'goal':goal_param},[]),
 'room.team.get':obj({'target':target},[]),
 'room.models.set':{**obj({'target':target,'avatar':ident,'lane':enum('chat','work'),'model':{'type':['string','null'],'minLength':1},'effort':{'anyOf':[effort,{'type':'null'}]},'if_revision':doc_revision},['target','avatar','lane']),'anyOf':[{'required':['model']},{'required':['effort']}]},
 'room.avatars.upsert':obj({'target':target,'id':avatar_id,'role':ident,'display':{'type':['string','null'],'minLength':1,'maxLength':64},'alias':{'type':['string','null'],'minLength':1,'maxLength':3,'pattern':'^[A-Za-z0-9]+$'},'if_revision':doc_revision},['target','id']),
 'room.avatars.remove':obj({'target':target,'id':avatar_id,'if_revision':doc_revision},['target','id']),
}
host_params.update(room_params)
params={**v1_params,**host_params}
d['request']={'oneOf':[obj({'id':ident,'method':{'const':m},'params':p}) for m,p in params.items()]}
d['reason']=obj({'code':ident,'message':s})
d['turn_result']=obj({'disposition':enum('started','queued','rejected','interrupted'),'turn_id':nullable,'reason':nullref('reason')})
d['replacement_result']=obj({'disposition':enum('started','rejected'),'turn_id':nullable,'reason':nullref('reason'),'interrupted_turn_id':nullable})
# reason, binding, allowed, written and detail are additive; v1 errors carry none of them.
d['error']=obj({'code':ident,'message':s,'delivery':enum('not_sent','unknown'),'reason':ident,'binding':ident,'allowed':names,'written':boolean,'detail':{'type':'object'}},['code','message','delivery'])
d['configure_result']=obj({'providers':{'type':'array','items':obj({'name':ident,'uuid':ident})},'models':obj({role:obj({'uuid':ident,'model_id':ident,'provider':ident}) for role in roles},[])},['providers','models'])
# capabilities stays the closed list of ten v1 methods; methods, features, events and limits are optional and open.
limits=obj({'frame_bytes':positive,'request_ids':positive,'text_bytes':positive,'attachments':obj({'count':positive,'pasted_bytes':positive,'total_bytes':positive},[]),'list':obj({'sessions':positive,'rooms':positive,'skills':positive,'usage_groups':positive,'usage_buckets':positive,'shelf_rows':positive,'reports':positive,'history_lines':positive,'trajectory_goals':positive,'trajectory_nodes':positive,'trajectory_edges':positive},[]),'room':obj({'ring_lines':positive,'ring_bytes':positive,'page_bytes':positive,'event_text_bytes':positive,'status_text_bytes':positive,'item_bytes':positive,'tail_lines':positive},[]),'skills':obj({'inline_zip_bytes':positive,'zip_bytes':positive,'entry_bytes':positive,'companion_bytes':positive,'files':positive,'depth':positive,'name_bytes':positive},[]),'config':obj({'lock_wait_ms':count},[]),'time':obj({'ipc_ms':positive,'room_start_ms':positive,'usage_query_ms':positive,'session_stop_ms':positive},[])},[])
d['hello']=obj({'type':{'const':'hello'},'protocol':{'const':1},'binary':obj({'path':ident,'version':ident}),'diagnostics':{'type':'array','items':s},'capabilities':{'type':'array','uniqueItems':True,'items':enum(*v1_params)},'methods':names,'features':names,'events':names,'limits':limits},['type','protocol','binary','diagnostics','capabilities'])
# Config and session DTOs. New enum-like strings are open; PublicRole is closed.
d['provider']=obj({'provider_uuid':ident,'name':s,'api_type':ident,'endpoint':s,'endpoint_redacted':boolean,'managed':boolean,'has_key':boolean,'key_state':ident,'model_count':count})
d['oauth']=obj({'oauth_uuid':ident,'name':s,'provider':s,'has_token':boolean,'expires_at':count})
d['model']=obj({'model_uuid':ident,'name':s,'model_id':s,'provider_uuid':s,'provider_kind':ident,'provider_name':nullable,'route':nullable,'roles':array(public_role,5,uniqueItems=True),'other_roles':boolean,'source_uuid':nullable})
d['role_route']=obj({'role':public_role,'origin':ident,'model_uuid':nullable,'source_uuid':nullable,'configured_model':nullable,'provider_uuid':nullable,'provider_name':nullable,'effective_model':nullable,'reason':nullable})
routes=array(ref('role_route'),5,minItems=5)
d['config_view']=obj({'revision':doc_revision,'file':ident,'view':ident,'providers':array(ref('provider'),256),'providers_truncated':boolean,'oauth':array(ref('oauth'),64),'oauth_truncated':boolean,'models':array(ref('model'),2000),'models_truncated':boolean,'roles':routes,'main_fallback':nullable})
limit_props={'max_output_tokens':count,'context_window_limit':count,'context_model_alias':s,'effective_window':{'type':['integer','null'],'minimum':0}}
d['limits']=obj(limit_props)
d['session_usage']=obj({'last_prompt_tokens':count,'last_cached_tokens':count,'output_tokens_total':count,'cost_total':{'type':'number','minimum':0},'context':{'anyOf':[obj({'prompt_tokens':count,'effective_window':count,'estimated':boolean,'cached_tokens':{'type':['integer','null'],'minimum':0},'drss_active':boolean}),{'type':'null'}]}})
d['session_info']=obj({'name':s,'effort':ident,'routes':routes,'limits':ref('limits'),'usage':nullref('session_usage'),'daemon':obj({'version':s,'ipc_protocol':count,'capabilities':array(ident,64)}),'observed_at_ms':count})
d['propagation']=obj({'bound':ident,'broadcast':ident,'rebind':ident,'rooms':ident,'skills':ident,'room':ident},[])
mutation={'config':ref('config_view'),'propagation':ref('propagation'),'session_info':ref('session_info')}
d['provider_delete_result']=obj({'dry_run':boolean,'removed':obj({'provider_uuid':ident,'models_removed':array(ident),'main_reset':boolean}),**mutation},['dry_run','removed','config','propagation'])
d['model_delete_result']=obj({'dry_run':boolean,'removed':obj({'model_uuid':ident,'roles_released':array(public_role,5,uniqueItems=True),'main_reset':boolean}),**mutation},['dry_run','removed','config','propagation'])
d['role_unassign_result']=obj({'changed':boolean,'previous_model_uuid':nullable,**mutation},['changed','previous_model_uuid','config','propagation'])
d['effort_options_result']=obj({'state':ident,'options':array(s,64),'selected':ident,'note':nullable,'main_model_uuid':nullable})
d['effort_result']=obj({'effort':ident,'main_model_uuid':nullable,'superseded':boolean,'session_info':ref('session_info')},['effort','main_model_uuid','superseded'])
d['limits_result']=obj({**limit_props,'session_info':ref('session_info')},list(limit_props))
# Sessions, usage and skills.
d['session_row']=obj({'session_id':ident,'name':nullable,'workdir':nullable,'kind':ident,'created_at':{'type':['integer','null']},'updated_at':{'type':['integer','null']},'live':boolean,'working':{'type':['boolean','null']},'locked':boolean,'bound':boolean,'parent_session_id':nullable,'room_session_id':nullable,'room_live':boolean})
d['sessions_list_result']=obj({'sessions':array(ref('session_row'),500),'next_cursor':nullable})
d['session_rename_result']=obj({'session':ref('session_row')})
d['session_delete_result']=obj({'deleted':{'const':True},'kind':ident,'orphaned_room_session_id':nullable})
d['session_stop_result']=obj({'stopped':{'const':True},'was_live':boolean})
d['transcript_message']=obj({'id':revision,'role':enum('system','user','assistant','tool'),'text':s,'created_at':count,'truncated':{'const':True}},['id','role','text','created_at'])
d['session_transcript_result']=obj({'session_id':ident,'live':boolean,'messages':array(ref('transcript_message'),500),'next_since':{'type':['integer','null'],'minimum':1}})
total_props={'calls':count,'tokens_in':count,'tokens_cached':count,'tokens_out':count,'cost':{'type':'number','minimum':0}}
d['usage_totals']=obj(total_props)
d['usage_group']=obj({**total_props,'key':s,'label':nullable,'role_kind':ident,'agent':nullable,'kind':nullable,'deleted':boolean},[*total_props,'key','label'])
d['usage_bucket']=obj({**total_props,'start_at':{'type':'integer'}})
d['usage_result']=obj({'from_at':{'type':'integer'},'to_at':{'type':'integer'},'ledger':ident,'totals':ref('usage_totals'),'groups':array(ref('usage_group'),500),'groups_truncated':boolean,'buckets':array(ref('usage_bucket'),1000)},['from_at','to_at','ledger','totals'])
skill_row_props={'name':ident,'scope':ident,'tier':ident,'description':s,'triggers':array(s,64),'triggers_truncated':boolean,'tags':array(s,64),'tags_truncated':boolean,'pinned':boolean,'editable':boolean,'generation':ident,'path':s,'effective':boolean,'shadowed_by':nullable}
d['skill_row']=obj(skill_row_props)
d['skill_detail']=obj({**skill_row_props,'instruction':s,'declared_tools':array(s,64),'declared_tools_truncated':boolean,'companion_files':array(s,1000),'companion_files_truncated':boolean,'structured_save_supported':boolean,'raw':nullable})
d['skills_list_result']=obj({'project_workdir':nullable,'skills':array(ref('skill_row'),1000),'diagnostics':array(obj({'path':s,'error':s}),200),'diagnostics_truncated':boolean,'next_cursor':nullable})
d['skill_file_result']=obj({'path':s,'text':s,'bytes':count})
d['skill_write_result']=obj({'skill':ref('skill_detail'),'propagation':ref('propagation')})
d['skill_update_result']=obj({'skill':ref('skill_detail'),'renamed':boolean,'propagation':ref('propagation')})
d['skill_remove_result']=obj({'removed':{'const':True},'now_effective':nullref('skill_row'),'propagation':ref('propagation')})
# Room DTOs. Enum-like strings are open, so they are typed as strings.
nullint={'type':['integer','null']}
any_int={'type':'integer'}
d['room_info']=obj({'session_id':ident,'parent_session_id':nullable,'parent_name':nullable,'name':nullable,'workdir':nullable,'live':boolean,'locked':boolean,'bound':boolean,'created_at':nullint,'updated_at':nullint})
d['room_line']=obj({'line_seq':line_seq,'kind':ident,'author':ident,'display':nullable,'alias':nullable,'lane':nullable,'audience':nullable,'text':nullable,'call_id':nullable,'tool':nullable,'args':nullable,'ok':{'type':['boolean','null']},'result':nullable,'replay':boolean,'origin':ident,'received_at_ms':count,'truncated':boolean})
room_counts=obj({'open':count,'verified':count,'returned':count,'reports':count})
d['lane_status']=obj({'avatar':ident,'display':s,'alias':s,'role':s,'lane':nullable,'phase':ident,'task':s,'started_at_ms':count,'last_event_at_ms':count,'steps':count,'last_tool':s,'error':s})
d['timeline_event']=obj({'at_ms':count,'avatar':s,'marker':s,'label':s})
d['room_status']=obj({'session_id':ident,'live':boolean,'fresh':boolean,'age_ms':{'type':['integer','null'],'minimum':0},'now_at_ms':count,'counts':room_counts,'counts_basis':ident,'paused':array(s,64),'paused_truncated':boolean,'lanes':array(ref('lane_status'),128),'lanes_truncated':boolean,'events':array(ref('timeline_event'),60),'digest':s,'timeline':s,'trajectory_outline':s,'shelf_fingerprint':s,'truncated':boolean})
nullseq={'anyOf':[line_seq,{'type':'null'}]}
d['attach_result']=obj({'room':ref('room_info'),'started':boolean,'daemon':obj({'version':nullable,'features':array(s,64),'template_revision':nullable}),'epoch':ident,'history':obj({'first_line_seq':nullseq,'last_line_seq':nullseq,'count':count,'replayed':count}),'tail':array(ref('room_line'),200),'status':nullref('room_status'),'roster':array(ident,64),'roster_truncated':boolean})
d['room_list_result']=obj({'rooms':array(ref('room_info'),500),'next_cursor':nullable})
d['room_close_result']=obj({'closed':{'const':True},'stopped':boolean})
d['room_send_result']=obj({'sent':{'const':True},'acked':boolean,'inserts':array(s,16),'echo_line_seq':line_seq})
d['room_sent_result']=obj({'sent':{'const':True}})
d['room_history_result']=obj({'epoch':ident,'first_line_seq':nullseq,'last_line_seq':nullseq,'lines':array(ref('room_line'),200),'has_more_before':boolean,'has_more_after':boolean})
shelf_row_props={'key':ident,'revision':any_int,'updated_at':any_int,'kind':s,'title':{'type':'string','maxLength':101},'status':s}
d['shelf_row']=obj(shelf_row_props)
d['shelf_item']=obj({**shelf_row_props,'content':s,'json':{'type':['object','null']},'truncated':boolean})
d['shelf_list_result']=obj({'rows':array(ref('shelf_row'),500),'total':count,'next_cursor':nullable,'fingerprint':s})
d['goal_node']=obj({'id':ident,'label':s,'kind':s,'status':nullable,'severity':nullable,'note':nullable,'resource_key':nullable,'revision':nullint,'rank':any_int,'row':any_int})
d['goal']=obj({'goal_key':ident,'counts':room_counts,'columns':count,'rows':count,'nodes':array(ref('goal_node'),2000),'nodes_truncated':boolean,'edges':array(obj({'from':ident,'to':ident,'rel':nullable}),8000),'edges_truncated':boolean})
d['trajectory_result']=obj({'goals':array(ref('goal'),200),'goals_truncated':boolean})
d['report']=obj({'key':ident,'revision':any_int,'updated_at':any_int,'title':s,'audience':s,'severity_summary':s,'go_no_go':s,'finding_keys':array(s,1000),'finding_keys_truncated':boolean,'markdown':nullable})
d['reports_result']=obj({'reports':array(ref('report'),100),'next_cursor':nullable})
spend_props={'calls':count,'prompt_tokens':count,'completion_tokens':count,'cost':{'type':'number','minimum':0}}
d['spend_entry']=obj(spend_props)
d['goal_spend']=obj({'goal_key':ident,'total':ref('spend_entry'),'by_avatar':array(obj({'avatar':ident,**spend_props}),64),'by_avatar_truncated':boolean})
d['spend_result']=obj({'goals':array(ref('goal_spend'),200),'goals_truncated':boolean,'ledger':obj({'ledger':ident,'totals':ref('usage_totals'),'by_avatar':array(obj({'avatar':ident,'totals':ref('usage_totals')}),64),'by_avatar_truncated':boolean})})
field=obj({'value':nullable,'origin':ident})
d['lane_view']=obj({'lane':ident,'provider_uuid':field,'model':field,'effort':field,'model_label':nullable,'resolves':boolean})
d['avatar_view']=obj({'id':ident,'role':ident,'defined_in':ident,'removable':boolean,'display':field,'display_effective':s,'alias':field,'alias_effective':s,'lanes':array(ref('lane_view'),2)})
role_lane=obj({'provider_uuid':nullable,'model':nullable,'effort':nullable})
d['role_view']=obj({'id':ident,'display':s,'shelf_allow':array(s),'skills_allow':{'anyOf':[array(s),{'type':'null'}]},'chat':role_lane,'work':role_lane,'defined_in':ident})
d['team']=obj({'target':ident,'revision':doc_revision,'layer_present':boolean,'template_revision':doc_revision,'template_pending_restart':boolean,'avatars':array(ref('avatar_view'),128),'avatars_truncated':boolean,'roles':array(ref('role_view'),64),'roles_truncated':boolean,'choices':array(obj({'id':ident,'name':s,'model_id':s,'provider_name':nullable,'provider_uuid':nullable}),2000),'choices_truncated':boolean,'choices_source':ident,'warnings':array(obj({'layer':ident,'path':s,'message':s}),100),'warnings_truncated':boolean})
team_write={'team':ref('team'),'applies':ident,'propagation':ref('propagation')}
d['team_write_result']=obj(team_write)
d['avatar_remove_result']=obj({**team_write,'reverted_to':nullable})
d['response']={'oneOf':[obj({'type':{'const':'response'},'id':ident,'result':{'oneOf':[obj({'state':ref('state'),'session_info':ref('session_info'),'warnings':array(obj({'code':ident,'message':s}),8)},['state']),ref('turn_result'),ref('replacement_result'),obj({'approved':boolean}),obj({'detached':{'const':True}}),ref('configure_result'),ref('config_view'),ref('provider_delete_result'),ref('model_delete_result'),ref('role_unassign_result'),ref('effort_options_result'),ref('effort_result'),ref('limits_result'),obj({'session_info':ref('session_info')}),ref('sessions_list_result'),ref('session_rename_result'),ref('session_delete_result'),ref('session_stop_result'),ref('session_transcript_result'),ref('usage_result'),ref('skills_list_result'),ref('skill_detail'),ref('skill_file_result'),ref('skill_write_result'),ref('skill_update_result'),ref('skill_remove_result'),ref('room_list_result'),ref('attach_result'),ref('room_close_result'),ref('room_send_result'),ref('room_sent_result'),ref('room_status'),ref('room_history_result'),ref('shelf_list_result'),ref('shelf_item'),ref('trajectory_result'),ref('reports_result'),ref('spend_result'),ref('team'),ref('team_write_result'),ref('avatar_remove_result')]}}),obj({'type':{'const':'response'},'id':ident,'error':ref('error')})]}
# Forbid only mutually exclusive envelope keys; additive fields remain permitted.
for variant in d['response']['oneOf']:
    variant['not']={'required':['error' if 'result' in variant['properties'] else 'result']}
# Generic result alternatives overlap because forward-compatible properties are allowed.
d['response']['oneOf'][0]['properties']['result']['anyOf']=d['response']['oneOf'][0]['properties']['result'].pop('oneOf')
d['event']={'oneOf':[obj({'type':{'const':'event'},'event':{'const':name},'seq':revision,**props}) for name,props in {
    'state':{'state':ref('state'),'resynced':boolean},
    'output_append':{'revision':revision,'offset':offset,'text':ident},
    'turn':{'revision':revision,'turn':nullref('turn')},
    # Room events: ROOM binding only, each tagged with the binding's epoch.
    'room.line':{'epoch':ident,'line':ref('room_line')},
    'room.delta':{'epoch':ident,'author':ident,'lane':ident,'kind':ident,'text':s},
    'room.status':{'epoch':ident,'status':ref('room_status')},
    'room.notice':{'epoch':ident,'code':ident,'text':s},
    'room.closed':{'epoch':ident,'session_id':ident,'reason':ident},
    'room.gap':{'epoch':ident,'after_line_seq':offset,'dropped_lines':count,'dropped_deltas':count},
}.items()]}
schema={'$schema':'https://json-schema.org/draft/2020-12/schema','$id':'https://koma.dev/sdk/protocol/v1/schema.json','title':'Koma public SDK JSONL protocol v1','oneOf':[ref(n) for n in ('request','hello','response','event')],'$defs':d}
Path(__file__).with_name('schema.json').write_text(json.dumps(schema,indent=2)+'\n')
