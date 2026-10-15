if(NOT DEFINED SHELL OR NOT DEFINED ROOT)
    message(FATAL_ERROR "SHELL and ROOT are required")
endif()
if(EXISTS "${ROOT}")
    message(FATAL_ERROR "plugin shell empty-store fixture unexpectedly exists: ${ROOT}")
endif()
execute_process(
    COMMAND "${SHELL}" "${ROOT}" list
    RESULT_VARIABLE result
    OUTPUT_VARIABLE output
    ERROR_VARIABLE diagnostic
)
if(NOT result EQUAL 0)
    message(FATAL_ERROR "plugin shell list failed: ${diagnostic}")
endif()
string(JSON schema ERROR_VARIABLE json_error GET "${output}" schema_version)
if(json_error OR NOT schema EQUAL 1)
    message(FATAL_ERROR "plugin shell schema is invalid: ${output}")
endif()
string(JSON plugin_count LENGTH "${output}" plugins)
string(JSON recovery_count LENGTH "${output}" recovery)
execute_process(COMMAND "${SHELL}" "${ROOT}" input-schemes
    RESULT_VARIABLE scheme_result OUTPUT_VARIABLE schemes ERROR_VARIABLE scheme_error)
if(NOT scheme_result EQUAL 0)
    message(FATAL_ERROR "empty input scheme discovery failed: ${scheme_error}")
endif()
string(JSON scheme_count LENGTH "${schemes}" schemes)
if(NOT scheme_count EQUAL 0)
    message(FATAL_ERROR "unexpected schemes in empty store: ${schemes}")
endif()
if(NOT plugin_count EQUAL 0 OR NOT recovery_count EQUAL 0)
    message(FATAL_ERROR "empty plugin store did not return empty arrays: ${output}")
endif()
execute_process(
    COMMAND "${SHELL}" "${ROOT}" cleanup 0 retained_staging "${ROOT}/missing" "" ""
    RESULT_VARIABLE invalid_result
)
if(invalid_result EQUAL 0)
    message(FATAL_ERROR "invalid recovery selection unexpectedly succeeded")
endif()
set(missing_package "${ROOT}/missing.owopkg")
execute_process(
    COMMAND "${SHELL}" "${ROOT}" install "${missing_package}"
    RESULT_VARIABLE install_result
    OUTPUT_VARIABLE install_output
    ERROR_VARIABLE install_diagnostic
)
if(NOT install_result EQUAL 2)
    message(FATAL_ERROR "missing plugin package returned ${install_result}: ${install_diagnostic}")
endif()
string(JSON install_schema ERROR_VARIABLE install_json_error GET "${install_output}" schema_version)
string(JSON install_ok ERROR_VARIABLE install_ok_error GET "${install_output}" ok)
string(JSON install_stage ERROR_VARIABLE install_stage_error GET "${install_output}" stage)
if(install_json_error OR install_ok_error OR install_stage_error OR
   NOT install_schema EQUAL 1 OR install_ok OR
   NOT install_stage STREQUAL "package_inspection")
    message(FATAL_ERROR "plugin install failure schema is invalid: ${install_output}")
endif()
if(EXISTS "${ROOT}")
    message(FATAL_ERROR "missing/untrusted install unexpectedly initialized the plugin store")
endif()
