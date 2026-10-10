# Providers, OAuth and models

## GUI
Connector contains Providers, OAuth accounts and Models. Add an API provider with its endpoint/key, or add an OAuth connection and complete the displayed login flow. Add a model using the corresponding provider or account. Then select a model in the composer. An OAuth account alone does not create a model selection. Availability is determined by the provider/account and model configuration.
## Scopes
Global providers and models belong to the installation. Session model assignments override global role assignments. Main drives normal turns; Planner can drive Plan mode. Sub-agent model choices are separate. koma-free is keyless; Help uses its own conversation and does not replace Main.
## TUI
/settings has provider, model and OAuth controls. /model changes a session or agent model; /free toggles the current session to koma-free. /effort sets supported reasoning effort.
## Recovery
Check the provider endpoint, key presence and login status before selecting the model again. Never paste credentials into Help. Expired OAuth may require reconnecting through Connector.
