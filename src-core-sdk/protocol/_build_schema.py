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
v1_params={'create':obj({'cwd':ident,'model_id':ident,'policy':ref('policy'),'external_approvals':boolean},['cwd']),'resume':obj({'session_id':ident,'policy':ref('policy'),'external_approvals':boolean},['session_id']),'state':obj({}),'submit':obj(input_props,['text']),'enqueue':obj(input_props,['text']),'interrupt':obj({'turn_id':ident}),'interrupt_and_submit':obj({'turn_id':ident,**input_props},['turn_id','text']),'approve':obj({'call_id':ident,'approve':boolean}),'configure':obj({'providers':{'type':'array','maxItems':32,'items':provider_in},'models':obj({role:assignment for role in roles},[])},[]),'detach':obj({})}
public_role=enum(*roles)
effort={'type':'string','minLength':1,'maxLength':64,'pattern':'^[^\\s\\x00-\\x1f\\x7f]+$'}
limit_params={'max_output_tokens':{'anyOf':[{'const':0},{'type':'integer','minimum':256,'maximum':1000000}]},'context_window_limit':{'anyOf':[{'const':0},{'type':'integer','minimum':1024,'maximum':100000000}]},'context_model_alias':{'type':'string','maxLength':200}}
host_params={'config.get':obj({'view':enum('global','session')},[]),'config.providers.delete':obj({'provider_uuid':ident,'if_revision':doc_revision,'dry_run':boolean},['provider_uuid']),'config.models.delete':obj({'model_uuid':ident,'if_revision':doc_revision,'dry_run':boolean},['model_uuid']),'config.roles.unassign':obj({'role':public_role,'scope':enum('global','session'),'if_revision':doc_revision},['role']),'config.effort.get':obj({'wait_ms':{'type':'integer','minimum':0,'maximum':10000}},[]),'config.effort.set':obj({'effort':effort,'if_main_model':nullable},['effort']),'config.limits.get':obj({}),'config.limits.set':{**obj(limit_params,[]),'anyOf':[{'required':[k]} for k in limit_params]},'sessions.info':obj({})}
params={**v1_params,**host_params}
d['request']={'oneOf':[obj({'id':ident,'method':{'const':m},'params':p}) for m,p in params.items()]}
d['reason']=obj({'code':ident,'message':s})
d['turn_result']=obj({'disposition':enum('started','queued','rejected','interrupted'),'turn_id':nullable,'reason':nullref('reason')})
d['replacement_result']=obj({'disposition':enum('started','rejected'),'turn_id':nullable,'reason':nullref('reason'),'interrupted_turn_id':nullable})
# reason, binding, allowed, written and detail are additive; v1 errors carry none of them.
d['error']=obj({'code':ident,'message':s,'delivery':enum('not_sent','unknown'),'reason':ident,'binding':ident,'allowed':names,'written':boolean,'detail':{'type':'object'}},['code','message','delivery'])
d['configure_result']=obj({'providers':{'type':'array','items':obj({'name':ident,'uuid':ident})},'models':obj({role:obj({'uuid':ident,'model_id':ident,'provider':ident}) for role in roles},[])},['providers','models'])
# capabilities stays the closed list of ten v1 methods; methods, features, events and limits are optional and open.
limits=obj({'frame_bytes':positive,'request_ids':positive,'text_bytes':positive,'attachments':obj({'count':positive,'pasted_bytes':positive,'total_bytes':positive},[]),'config':obj({'lock_wait_ms':count},[]),'time':obj({'ipc_ms':positive},[])},[])
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
d['propagation']=obj({'bound':ident,'broadcast':ident,'rebind':ident,'rooms':ident},[])
mutation={'config':ref('config_view'),'propagation':ref('propagation'),'session_info':ref('session_info')}
d['provider_delete_result']=obj({'dry_run':boolean,'removed':obj({'provider_uuid':ident,'models_removed':array(ident),'main_reset':boolean}),**mutation},['dry_run','removed','config','propagation'])
d['model_delete_result']=obj({'dry_run':boolean,'removed':obj({'model_uuid':ident,'roles_released':array(public_role,5,uniqueItems=True),'main_reset':boolean}),**mutation},['dry_run','removed','config','propagation'])
d['role_unassign_result']=obj({'changed':boolean,'previous_model_uuid':nullable,**mutation},['changed','previous_model_uuid','config','propagation'])
d['effort_options_result']=obj({'state':ident,'options':array(s,64),'selected':ident,'note':nullable,'main_model_uuid':nullable})
d['effort_result']=obj({'effort':ident,'main_model_uuid':nullable,'superseded':boolean,'session_info':ref('session_info')},['effort','main_model_uuid','superseded'])
d['limits_result']=obj({**limit_props,'session_info':ref('session_info')},list(limit_props))
d['response']={'oneOf':[obj({'type':{'const':'response'},'id':ident,'result':{'oneOf':[obj({'state':ref('state'),'session_info':ref('session_info')},['state']),ref('turn_result'),ref('replacement_result'),obj({'approved':boolean}),obj({'detached':{'const':True}}),ref('configure_result'),ref('config_view'),ref('provider_delete_result'),ref('model_delete_result'),ref('role_unassign_result'),ref('effort_options_result'),ref('effort_result'),ref('limits_result'),obj({'session_info':ref('session_info')})]}}),obj({'type':{'const':'response'},'id':ident,'error':ref('error')})]}
# Forbid only mutually exclusive envelope keys; additive fields remain permitted.
for variant in d['response']['oneOf']:
    variant['not']={'required':['error' if 'result' in variant['properties'] else 'result']}
# Generic result alternatives overlap because forward-compatible properties are allowed.
d['response']['oneOf'][0]['properties']['result']['anyOf']=d['response']['oneOf'][0]['properties']['result'].pop('oneOf')
d['event']={'oneOf':[obj({'type':{'const':'event'},'event':{'const':name},'seq':revision,**props}) for name,props in {
    'state':{'state':ref('state'),'resynced':boolean},
    'output_append':{'revision':revision,'offset':offset,'text':ident},
    'turn':{'revision':revision,'turn':nullref('turn')},
}.items()]}
schema={'$schema':'https://json-schema.org/draft/2020-12/schema','$id':'https://koma.dev/sdk/protocol/v1/schema.json','title':'Koma public SDK JSONL protocol v1','oneOf':[ref(n) for n in ('request','hello','response','event')],'$defs':d}
Path(__file__).with_name('schema.json').write_text(json.dumps(schema,indent=2)+'\n')
