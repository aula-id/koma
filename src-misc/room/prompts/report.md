Reporting for goal {{goal}}.
{{gaps}}
Findings to cover: {{findings}}
For each finding without a detailed technical report, put_report an audience=technical report under reports/ with the finding_key set: summary, impact, evidence keys, repro steps, residual risk. Then put_report one audience=manager executive summary whose finding_keys cover every finding. Then call group_task_complete once with every reports/ key you wrote.
put_finding never writes reports/; you are the only writer there. Confirmed identifiers only; never invent a CVE or CWE.
