using System;
using System.Diagnostics;
using System.Runtime.InteropServices;
using System.Threading.Tasks;
using Google.Protobuf;
using NPitaya.Metrics;
using NPitaya.Models;
using NPitaya.Serializer;
using NPitaya.Protos;
using static NPitaya.Utils.Utils;

namespace NPitaya
{
    public partial class PitayaCluster
    {
        private static async Task HandleIncomingRpc(IntPtr cRpcPtr)
        {
            return;
            // var sw = Stopwatch.StartNew();
            // var res = new MemoryBuffer();
            // bool success = false;
            // string route = "";
            // IntPtr resPtr;
            // try
            // {
                // var cRpc = (CRpc) Marshal.PtrToStructure(cRpcPtr, typeof(CRpc));
                // var req = BuildRequestData(cRpc.reqBufferPtr);
                // route = req.Msg.Route;
                // var res = await RPCCbFuncImpl(req, sw);
                // success = true;
            // }
            // catch (Exception e)
            // {
                // var innerMostException = e;
                // while (innerMostException.InnerException != null)
                    // innerMostException = innerMostException.InnerException;
//
                // Logger.Error("Exception thrown in handler, error:{0}",
                    // innerMostException.Message); // TODO externalize method and only print stacktrace when debug
// #if NPITAYA_DEBUG
                // If we are compiling with a Debug define, we want to print a stacktrace whenever a route
                // throws an exception.
                // Logger.Error("Stacktrace: {0}", innerMostException.StackTrace);
// #endif

                // var protosResponse = GetErrorResponse("PIT-500", innerMostException.Message);
                // var responseBytes = protosResponse.ToByteArray();
                // res.data = ByteArrayToIntPtr(responseBytes);
                // res.size = responseBytes.Length;
            // }
            // finally
            // {
                // resPtr = Marshal.AllocHGlobal(Marshal.SizeOf(res));
                // Marshal.StructureToPtr(res, resPtr, false);

                // tfg_pitc_FinishRpcCall(resPtr, cRpcPtr);

                // Marshal.FreeHGlobal(res.data);
                // Marshal.FreeHGlobal(resPtr);
                // if (success)
                // {
                    // MetricsReporters.ReportTimer(Metrics.Constants.Status.success.ToString(), route,
                        // "handler", "", sw);
                // }
                // else
                // {
                    // MetricsReporters.ReportTimer(Metrics.Constants.Status.fail.ToString(), route,
                        // "handler", "PIT-500", sw);
                // }
            // }
        }

        private static Request BuildRequestData(IntPtr reqBufferPtr)
        {
            var reqBuffer = (MemoryBuffer) Marshal.PtrToStructure(reqBufferPtr, typeof(MemoryBuffer));

            Request req = new Request();
            req.MergeFrom(new CodedInputStream(reqBuffer.GetData()));
            return req;
        }

        private static async Task<Response> RPCCbFuncImpl(Request req, Stopwatch sw)
        {
            Response response;
            switch (req.Type)
            {
                case RPCType.User:
                    response = await HandleRpc(req, RPCType.User);
                    break;
                case RPCType.Sys:
                    response = await HandleRpc(req, RPCType.Sys);
                    break;
                default:
                    throw new Exception($"invalid rpc type, argument:{req.Type}");
            }
            return response;
        }

        internal static async Task<Response> HandleRpc(Protos.Request req, RPCType type)
        {
            byte[] data = req.Msg.Data.ToByteArray();
            Route route = Route.FromString(req.Msg.Route);

            string handlerName = $"{route.service}.{route.method}";

            PitayaSession s = null;
            var response = new Response();

            RemoteMethod handler;
            if (type == RPCType.Sys)
            {
                s = new Models.PitayaSession(req.Session, req.FrontendID);
                if (!HandlersDict.ContainsKey(handlerName))
                {
                    response = GetErrorResponse("PIT-404",
                        $"remote/handler not found! remote/handler name: {handlerName}");
                    return response;
                }

                handler = HandlersDict[handlerName];
            }
            else
            {
                if (!RemotesDict.ContainsKey(handlerName))
                {
                    response = GetErrorResponse("PIT-404",
                        $"remote/handler not found! remote/handler name: {handlerName}");
                    return response;
                }

                handler = RemotesDict[handlerName];
            }

            Task ans;
            if (handler.ArgType != null)
            {
                var arg = _serializer.Unmarshal(data, handler.ArgType);
                if (type == RPCType.Sys)
                    ans = handler.Method.Invoke(handler.Obj, new[] {s, arg}) as Task;
                else
                    ans = handler.Method.Invoke(handler.Obj, new[] {arg}) as Task;
            }
            else
            {
                if (type == RPCType.Sys)
                    ans = handler.Method.Invoke(handler.Obj, new object[] {s}) as Task;
                else
                    ans = handler.Method.Invoke(handler.Obj, new object[] { }) as Task;
            }

            await ans;
            byte[] ansBytes;

            if (handler.ReturnType != typeof(void))
            {
                ansBytes = SerializerUtils.SerializeOrRaw(ans.GetType().
                    GetProperty("Result")
                    ?.GetValue(ans), _serializer);
            }
            else
            {
                ansBytes = new byte[]{};
            }
            response.Data = ByteString.CopyFrom(ansBytes);
            return response;
        }
    }
}