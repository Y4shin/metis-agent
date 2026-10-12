# \DefaultApi

All URIs are relative to *http://localhost*

Method | HTTP request | Description
------------- | ------------- | -------------
[**echo**](DefaultApi.md#echo) | **POST** /v1/echo | Echo a message back with a server timestamp.
[**stream_events**](DefaultApi.md#stream_events) | **GET** /v1/events | Server-sent event stream with resumable cursor.



## echo

> models::EchoResponse echo(echo_request)
Echo a message back with a server timestamp.

### Parameters


Name | Type | Description  | Required | Notes
------------- | ------------- | ------------- | ------------- | -------------
**echo_request** | [**EchoRequest**](EchoRequest.md) |  | [required] |

### Return type

[**models::EchoResponse**](EchoResponse.md)

### Authorization

No authorization required

### HTTP request headers

- **Content-Type**: application/json
- **Accept**: application/json

[[Back to top]](#) [[Back to API list]](../README.md#documentation-for-api-endpoints) [[Back to Model list]](../README.md#documentation-for-models) [[Back to README]](../README.md)


## stream_events

> models::Event stream_events(last_event_id)
Server-sent event stream with resumable cursor.

### Parameters


Name | Type | Description  | Required | Notes
------------- | ------------- | ------------- | ------------- | -------------
**last_event_id** | Option<**String**> | Cursor of the last event the client saw, for resume. |  |

### Return type

[**models::Event**](Event.md)

### Authorization

No authorization required

### HTTP request headers

- **Content-Type**: Not defined
- **Accept**: text/event-stream

[[Back to top]](#) [[Back to API list]](../README.md#documentation-for-api-endpoints) [[Back to Model list]](../README.md#documentation-for-models) [[Back to README]](../README.md)

