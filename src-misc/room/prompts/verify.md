Verification for goal {{goal}}.
Re-check these claim and evidence revisions independently:
{{items}}
Record a verdict for each claim revision. Publish a finding for each confirmed claim. Cite any re-check evidence you wrote in the verdict's recheck_evidence_keys. Then call group_task_complete once with the references you reviewed and produced, and say in its result which finding keys are confirmed and ready for the Reporter.
Each item has a code_check: grounded (a program found the cited code; the bug is still yours to judge) or unproven (nothing could be checked, so check the code yourself). If you cannot establish a claim either way, return it with the evidence gap; dismiss only what you have shown is wrong.
