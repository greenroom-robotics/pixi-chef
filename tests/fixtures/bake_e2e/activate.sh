source /dev/null
if [[ -z "${BAKE_E2E_MARK:-}" ]]; then
    export BAKE_E2E_MARK=active
fi
