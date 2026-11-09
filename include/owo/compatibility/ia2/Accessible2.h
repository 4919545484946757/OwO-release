

/* this ALWAYS GENERATED file contains the definitions for the interfaces */


 /* File created by MIDL compiler version 8.01.0628 */
/* at Tue Jan 19 11:14:07 2038
 */
/* Compiler settings for env/compatibility/ia2/Accessible2.idl:
    Oicf, W1, Zp8, env=Win64 (32b run), target_arch=AMD64 8.01.0628 
    protocol : dce , ms_ext, c_ext, robust
    error checks: allocation ref bounds_check enum stub_data 
    VC __declspec() decoration level: 
         __declspec(uuid()), __declspec(selectany), __declspec(novtable)
         DECLSPEC_UUID(), MIDL_INTERFACE()
*/
/* @@MIDL_FILE_HEADING(  ) */

#pragma warning( disable: 4049 )  /* more than 64k source lines */


/* verify that the <rpcndr.h> version is high enough to compile this file*/
#ifndef __REQUIRED_RPCNDR_H_VERSION__
#define __REQUIRED_RPCNDR_H_VERSION__ 475
#endif

#include "rpc.h"
#include "rpcndr.h"

#ifndef __RPCNDR_H_VERSION__
#error this stub requires an updated version of <rpcndr.h>
#endif /* __RPCNDR_H_VERSION__ */

#ifndef COM_NO_WINDOWS_H
#include "windows.h"
#include "ole2.h"
#endif /*COM_NO_WINDOWS_H*/

#ifndef __Accessible2_h__
#define __Accessible2_h__

#if defined(_MSC_VER) && (_MSC_VER >= 1020)
#pragma once
#endif

#ifndef DECLSPEC_XFGVIRT
#if defined(_CONTROL_FLOW_GUARD_XFG)
#define DECLSPEC_XFGVIRT(base, func) __declspec(xfg_virtual(base, func))
#else
#define DECLSPEC_XFGVIRT(base, func)
#endif
#endif

/* Forward Declarations */ 

#ifndef __IAccessible2_FWD_DEFINED__
#define __IAccessible2_FWD_DEFINED__
typedef interface IAccessible2 IAccessible2;

#endif 	/* __IAccessible2_FWD_DEFINED__ */


/* header files for imported files */
#include "objidl.h"
#include "oaidl.h"
#include "oleacc.h"
#include "AccessibleRelation.h"
#include "AccessibleStates.h"
#include "IA2CommonTypes.h"

#ifdef __cplusplus
extern "C"{
#endif 


/* interface __MIDL_itf_Accessible2_0000_0000 */
/* [local] */ 

typedef struct IA2Locale
    {
    BSTR language;
    BSTR country;
    BSTR variant;
    } 	IA2Locale;



extern RPC_IF_HANDLE __MIDL_itf_Accessible2_0000_0000_v0_0_c_ifspec;
extern RPC_IF_HANDLE __MIDL_itf_Accessible2_0000_0000_v0_0_s_ifspec;

#ifndef __IAccessible2_INTERFACE_DEFINED__
#define __IAccessible2_INTERFACE_DEFINED__

/* interface IAccessible2 */
/* [uuid][object] */ 


EXTERN_C const IID IID_IAccessible2;

#if defined(__cplusplus) && !defined(CINTERFACE)
    
    MIDL_INTERFACE("E89F726E-C4F4-4c19-BB19-B647D7FA8478")
    IAccessible2 : public IAccessible
    {
    public:
        virtual /* [propget] */ HRESULT STDMETHODCALLTYPE get_nRelations( 
            /* [retval][out] */ long *nRelations) = 0;
        
        virtual /* [propget] */ HRESULT STDMETHODCALLTYPE get_relation( 
            /* [in] */ long relationIndex,
            /* [retval][out] */ IAccessibleRelation **relation) = 0;
        
        virtual /* [propget] */ HRESULT STDMETHODCALLTYPE get_relations( 
            /* [in] */ long maxRelations,
            /* [length_is][size_is][out] */ IAccessibleRelation **relations,
            /* [retval][out] */ long *nRelations) = 0;
        
        virtual HRESULT STDMETHODCALLTYPE role( 
            /* [retval][out] */ long *role) = 0;
        
        virtual HRESULT STDMETHODCALLTYPE scrollTo( 
            /* [in] */ enum IA2ScrollType scrollType) = 0;
        
        virtual HRESULT STDMETHODCALLTYPE scrollToPoint( 
            /* [in] */ enum IA2CoordinateType coordinateType,
            /* [in] */ long x,
            /* [in] */ long y) = 0;
        
        virtual /* [propget] */ HRESULT STDMETHODCALLTYPE get_groupPosition( 
            /* [out] */ long *groupLevel,
            /* [out] */ long *similarItemsInGroup,
            /* [retval][out] */ long *positionInGroup) = 0;
        
        virtual /* [propget] */ HRESULT STDMETHODCALLTYPE get_states( 
            /* [retval][out] */ AccessibleStates *states) = 0;
        
        virtual /* [propget] */ HRESULT STDMETHODCALLTYPE get_extendedRole( 
            /* [retval][out] */ BSTR *extendedRole) = 0;
        
        virtual /* [propget] */ HRESULT STDMETHODCALLTYPE get_localizedExtendedRole( 
            /* [retval][out] */ BSTR *localizedExtendedRole) = 0;
        
        virtual /* [propget] */ HRESULT STDMETHODCALLTYPE get_nExtendedStates( 
            /* [retval][out] */ long *nExtendedStates) = 0;
        
        virtual /* [propget] */ HRESULT STDMETHODCALLTYPE get_extendedStates( 
            /* [in] */ long maxExtendedStates,
            /* [length_is][length_is][size_is][size_is][out] */ BSTR **extendedStates,
            /* [retval][out] */ long *nExtendedStates) = 0;
        
        virtual /* [propget] */ HRESULT STDMETHODCALLTYPE get_localizedExtendedStates( 
            /* [in] */ long maxLocalizedExtendedStates,
            /* [length_is][length_is][size_is][size_is][out] */ BSTR **localizedExtendedStates,
            /* [retval][out] */ long *nLocalizedExtendedStates) = 0;
        
        virtual /* [propget] */ HRESULT STDMETHODCALLTYPE get_uniqueID( 
            /* [retval][out] */ long *uniqueID) = 0;
        
        virtual /* [propget] */ HRESULT STDMETHODCALLTYPE get_windowHandle( 
            /* [retval][out] */ HWND *windowHandle) = 0;
        
        virtual /* [propget] */ HRESULT STDMETHODCALLTYPE get_indexInParent( 
            /* [retval][out] */ long *indexInParent) = 0;
        
        virtual /* [propget] */ HRESULT STDMETHODCALLTYPE get_locale( 
            /* [retval][out] */ IA2Locale *locale) = 0;
        
        virtual /* [propget] */ HRESULT STDMETHODCALLTYPE get_attributes( 
            /* [retval][out] */ BSTR *attributes) = 0;
        
    };
    
    
#else 	/* C style interface */

    typedef struct IAccessible2Vtbl
    {
        BEGIN_INTERFACE
        
        DECLSPEC_XFGVIRT(IUnknown, QueryInterface)
        HRESULT ( STDMETHODCALLTYPE *QueryInterface )( 
            IAccessible2 * This,
            /* [in] */ REFIID riid,
            /* [annotation][iid_is][out] */ 
            _COM_Outptr_  void **ppvObject);
        
        DECLSPEC_XFGVIRT(IUnknown, AddRef)
        ULONG ( STDMETHODCALLTYPE *AddRef )( 
            IAccessible2 * This);
        
        DECLSPEC_XFGVIRT(IUnknown, Release)
        ULONG ( STDMETHODCALLTYPE *Release )( 
            IAccessible2 * This);
        
        DECLSPEC_XFGVIRT(IDispatch, GetTypeInfoCount)
        HRESULT ( STDMETHODCALLTYPE *GetTypeInfoCount )( 
            IAccessible2 * This,
            /* [out] */ UINT *pctinfo);
        
        DECLSPEC_XFGVIRT(IDispatch, GetTypeInfo)
        HRESULT ( STDMETHODCALLTYPE *GetTypeInfo )( 
            IAccessible2 * This,
            /* [in] */ UINT iTInfo,
            /* [in] */ LCID lcid,
            /* [out] */ ITypeInfo **ppTInfo);
        
        DECLSPEC_XFGVIRT(IDispatch, GetIDsOfNames)
        HRESULT ( STDMETHODCALLTYPE *GetIDsOfNames )( 
            IAccessible2 * This,
            /* [in] */ REFIID riid,
            /* [size_is][in] */ LPOLESTR *rgszNames,
            /* [range][in] */ UINT cNames,
            /* [in] */ LCID lcid,
            /* [size_is][out] */ DISPID *rgDispId);
        
        DECLSPEC_XFGVIRT(IDispatch, Invoke)
        /* [local] */ HRESULT ( STDMETHODCALLTYPE *Invoke )( 
            IAccessible2 * This,
            /* [annotation][in] */ 
            _In_  DISPID dispIdMember,
            /* [annotation][in] */ 
            _In_  REFIID riid,
            /* [annotation][in] */ 
            _In_  LCID lcid,
            /* [annotation][in] */ 
            _In_  WORD wFlags,
            /* [annotation][out][in] */ 
            _In_  DISPPARAMS *pDispParams,
            /* [annotation][out] */ 
            _Out_opt_  VARIANT *pVarResult,
            /* [annotation][out] */ 
            _Out_opt_  EXCEPINFO *pExcepInfo,
            /* [annotation][out] */ 
            _Out_opt_  UINT *puArgErr);
        
        DECLSPEC_XFGVIRT(IAccessible, get_accParent)
        /* [id][propget][hidden] */ HRESULT ( STDMETHODCALLTYPE *get_accParent )( 
            IAccessible2 * This,
            /* [retval][out] */ IDispatch **ppdispParent);
        
        DECLSPEC_XFGVIRT(IAccessible, get_accChildCount)
        /* [id][propget][hidden] */ HRESULT ( STDMETHODCALLTYPE *get_accChildCount )( 
            IAccessible2 * This,
            /* [retval][out] */ long *pcountChildren);
        
        DECLSPEC_XFGVIRT(IAccessible, get_accChild)
        /* [id][propget][hidden] */ HRESULT ( STDMETHODCALLTYPE *get_accChild )( 
            IAccessible2 * This,
            /* [in] */ VARIANT varChild,
            /* [retval][out] */ IDispatch **ppdispChild);
        
        DECLSPEC_XFGVIRT(IAccessible, get_accName)
        /* [id][propget][hidden] */ HRESULT ( STDMETHODCALLTYPE *get_accName )( 
            IAccessible2 * This,
            /* [optional][in] */ VARIANT varChild,
            /* [retval][out] */ BSTR *pszName);
        
        DECLSPEC_XFGVIRT(IAccessible, get_accValue)
        /* [id][propget][hidden] */ HRESULT ( STDMETHODCALLTYPE *get_accValue )( 
            IAccessible2 * This,
            /* [optional][in] */ VARIANT varChild,
            /* [retval][out] */ BSTR *pszValue);
        
        DECLSPEC_XFGVIRT(IAccessible, get_accDescription)
        /* [id][propget][hidden] */ HRESULT ( STDMETHODCALLTYPE *get_accDescription )( 
            IAccessible2 * This,
            /* [optional][in] */ VARIANT varChild,
            /* [retval][out] */ BSTR *pszDescription);
        
        DECLSPEC_XFGVIRT(IAccessible, get_accRole)
        /* [id][propget][hidden] */ HRESULT ( STDMETHODCALLTYPE *get_accRole )( 
            IAccessible2 * This,
            /* [optional][in] */ VARIANT varChild,
            /* [retval][out] */ VARIANT *pvarRole);
        
        DECLSPEC_XFGVIRT(IAccessible, get_accState)
        /* [id][propget][hidden] */ HRESULT ( STDMETHODCALLTYPE *get_accState )( 
            IAccessible2 * This,
            /* [optional][in] */ VARIANT varChild,
            /* [retval][out] */ VARIANT *pvarState);
        
        DECLSPEC_XFGVIRT(IAccessible, get_accHelp)
        /* [id][propget][hidden] */ HRESULT ( STDMETHODCALLTYPE *get_accHelp )( 
            IAccessible2 * This,
            /* [optional][in] */ VARIANT varChild,
            /* [retval][out] */ BSTR *pszHelp);
        
        DECLSPEC_XFGVIRT(IAccessible, get_accHelpTopic)
        /* [id][propget][hidden] */ HRESULT ( STDMETHODCALLTYPE *get_accHelpTopic )( 
            IAccessible2 * This,
            /* [out] */ BSTR *pszHelpFile,
            /* [optional][in] */ VARIANT varChild,
            /* [retval][out] */ long *pidTopic);
        
        DECLSPEC_XFGVIRT(IAccessible, get_accKeyboardShortcut)
        /* [id][propget][hidden] */ HRESULT ( STDMETHODCALLTYPE *get_accKeyboardShortcut )( 
            IAccessible2 * This,
            /* [optional][in] */ VARIANT varChild,
            /* [retval][out] */ BSTR *pszKeyboardShortcut);
        
        DECLSPEC_XFGVIRT(IAccessible, get_accFocus)
        /* [id][propget][hidden] */ HRESULT ( STDMETHODCALLTYPE *get_accFocus )( 
            IAccessible2 * This,
            /* [retval][out] */ VARIANT *pvarChild);
        
        DECLSPEC_XFGVIRT(IAccessible, get_accSelection)
        /* [id][propget][hidden] */ HRESULT ( STDMETHODCALLTYPE *get_accSelection )( 
            IAccessible2 * This,
            /* [retval][out] */ VARIANT *pvarChildren);
        
        DECLSPEC_XFGVIRT(IAccessible, get_accDefaultAction)
        /* [id][propget][hidden] */ HRESULT ( STDMETHODCALLTYPE *get_accDefaultAction )( 
            IAccessible2 * This,
            /* [optional][in] */ VARIANT varChild,
            /* [retval][out] */ BSTR *pszDefaultAction);
        
        DECLSPEC_XFGVIRT(IAccessible, accSelect)
        /* [id][hidden] */ HRESULT ( STDMETHODCALLTYPE *accSelect )( 
            IAccessible2 * This,
            /* [in] */ long flagsSelect,
            /* [optional][in] */ VARIANT varChild);
        
        DECLSPEC_XFGVIRT(IAccessible, accLocation)
        /* [id][hidden] */ HRESULT ( STDMETHODCALLTYPE *accLocation )( 
            IAccessible2 * This,
            /* [out] */ long *pxLeft,
            /* [out] */ long *pyTop,
            /* [out] */ long *pcxWidth,
            /* [out] */ long *pcyHeight,
            /* [optional][in] */ VARIANT varChild);
        
        DECLSPEC_XFGVIRT(IAccessible, accNavigate)
        /* [id][hidden] */ HRESULT ( STDMETHODCALLTYPE *accNavigate )( 
            IAccessible2 * This,
            /* [in] */ long navDir,
            /* [optional][in] */ VARIANT varStart,
            /* [retval][out] */ VARIANT *pvarEndUpAt);
        
        DECLSPEC_XFGVIRT(IAccessible, accHitTest)
        /* [id][hidden] */ HRESULT ( STDMETHODCALLTYPE *accHitTest )( 
            IAccessible2 * This,
            /* [in] */ long xLeft,
            /* [in] */ long yTop,
            /* [retval][out] */ VARIANT *pvarChild);
        
        DECLSPEC_XFGVIRT(IAccessible, accDoDefaultAction)
        /* [id][hidden] */ HRESULT ( STDMETHODCALLTYPE *accDoDefaultAction )( 
            IAccessible2 * This,
            /* [optional][in] */ VARIANT varChild);
        
        DECLSPEC_XFGVIRT(IAccessible, put_accName)
        /* [id][propput][hidden] */ HRESULT ( STDMETHODCALLTYPE *put_accName )( 
            IAccessible2 * This,
            /* [optional][in] */ VARIANT varChild,
            /* [in] */ BSTR szName);
        
        DECLSPEC_XFGVIRT(IAccessible, put_accValue)
        /* [id][propput][hidden] */ HRESULT ( STDMETHODCALLTYPE *put_accValue )( 
            IAccessible2 * This,
            /* [optional][in] */ VARIANT varChild,
            /* [in] */ BSTR szValue);
        
        DECLSPEC_XFGVIRT(IAccessible2, get_nRelations)
        /* [propget] */ HRESULT ( STDMETHODCALLTYPE *get_nRelations )( 
            IAccessible2 * This,
            /* [retval][out] */ long *nRelations);
        
        DECLSPEC_XFGVIRT(IAccessible2, get_relation)
        /* [propget] */ HRESULT ( STDMETHODCALLTYPE *get_relation )( 
            IAccessible2 * This,
            /* [in] */ long relationIndex,
            /* [retval][out] */ IAccessibleRelation **relation);
        
        DECLSPEC_XFGVIRT(IAccessible2, get_relations)
        /* [propget] */ HRESULT ( STDMETHODCALLTYPE *get_relations )( 
            IAccessible2 * This,
            /* [in] */ long maxRelations,
            /* [length_is][size_is][out] */ IAccessibleRelation **relations,
            /* [retval][out] */ long *nRelations);
        
        DECLSPEC_XFGVIRT(IAccessible2, role)
        HRESULT ( STDMETHODCALLTYPE *role )( 
            IAccessible2 * This,
            /* [retval][out] */ long *role);
        
        DECLSPEC_XFGVIRT(IAccessible2, scrollTo)
        HRESULT ( STDMETHODCALLTYPE *scrollTo )( 
            IAccessible2 * This,
            /* [in] */ enum IA2ScrollType scrollType);
        
        DECLSPEC_XFGVIRT(IAccessible2, scrollToPoint)
        HRESULT ( STDMETHODCALLTYPE *scrollToPoint )( 
            IAccessible2 * This,
            /* [in] */ enum IA2CoordinateType coordinateType,
            /* [in] */ long x,
            /* [in] */ long y);
        
        DECLSPEC_XFGVIRT(IAccessible2, get_groupPosition)
        /* [propget] */ HRESULT ( STDMETHODCALLTYPE *get_groupPosition )( 
            IAccessible2 * This,
            /* [out] */ long *groupLevel,
            /* [out] */ long *similarItemsInGroup,
            /* [retval][out] */ long *positionInGroup);
        
        DECLSPEC_XFGVIRT(IAccessible2, get_states)
        /* [propget] */ HRESULT ( STDMETHODCALLTYPE *get_states )( 
            IAccessible2 * This,
            /* [retval][out] */ AccessibleStates *states);
        
        DECLSPEC_XFGVIRT(IAccessible2, get_extendedRole)
        /* [propget] */ HRESULT ( STDMETHODCALLTYPE *get_extendedRole )( 
            IAccessible2 * This,
            /* [retval][out] */ BSTR *extendedRole);
        
        DECLSPEC_XFGVIRT(IAccessible2, get_localizedExtendedRole)
        /* [propget] */ HRESULT ( STDMETHODCALLTYPE *get_localizedExtendedRole )( 
            IAccessible2 * This,
            /* [retval][out] */ BSTR *localizedExtendedRole);
        
        DECLSPEC_XFGVIRT(IAccessible2, get_nExtendedStates)
        /* [propget] */ HRESULT ( STDMETHODCALLTYPE *get_nExtendedStates )( 
            IAccessible2 * This,
            /* [retval][out] */ long *nExtendedStates);
        
        DECLSPEC_XFGVIRT(IAccessible2, get_extendedStates)
        /* [propget] */ HRESULT ( STDMETHODCALLTYPE *get_extendedStates )( 
            IAccessible2 * This,
            /* [in] */ long maxExtendedStates,
            /* [length_is][length_is][size_is][size_is][out] */ BSTR **extendedStates,
            /* [retval][out] */ long *nExtendedStates);
        
        DECLSPEC_XFGVIRT(IAccessible2, get_localizedExtendedStates)
        /* [propget] */ HRESULT ( STDMETHODCALLTYPE *get_localizedExtendedStates )( 
            IAccessible2 * This,
            /* [in] */ long maxLocalizedExtendedStates,
            /* [length_is][length_is][size_is][size_is][out] */ BSTR **localizedExtendedStates,
            /* [retval][out] */ long *nLocalizedExtendedStates);
        
        DECLSPEC_XFGVIRT(IAccessible2, get_uniqueID)
        /* [propget] */ HRESULT ( STDMETHODCALLTYPE *get_uniqueID )( 
            IAccessible2 * This,
            /* [retval][out] */ long *uniqueID);
        
        DECLSPEC_XFGVIRT(IAccessible2, get_windowHandle)
        /* [propget] */ HRESULT ( STDMETHODCALLTYPE *get_windowHandle )( 
            IAccessible2 * This,
            /* [retval][out] */ HWND *windowHandle);
        
        DECLSPEC_XFGVIRT(IAccessible2, get_indexInParent)
        /* [propget] */ HRESULT ( STDMETHODCALLTYPE *get_indexInParent )( 
            IAccessible2 * This,
            /* [retval][out] */ long *indexInParent);
        
        DECLSPEC_XFGVIRT(IAccessible2, get_locale)
        /* [propget] */ HRESULT ( STDMETHODCALLTYPE *get_locale )( 
            IAccessible2 * This,
            /* [retval][out] */ IA2Locale *locale);
        
        DECLSPEC_XFGVIRT(IAccessible2, get_attributes)
        /* [propget] */ HRESULT ( STDMETHODCALLTYPE *get_attributes )( 
            IAccessible2 * This,
            /* [retval][out] */ BSTR *attributes);
        
        END_INTERFACE
    } IAccessible2Vtbl;

    interface IAccessible2
    {
        CONST_VTBL struct IAccessible2Vtbl *lpVtbl;
    };

    

#ifdef COBJMACROS


#define IAccessible2_QueryInterface(This,riid,ppvObject)	\
    ( (This)->lpVtbl -> QueryInterface(This,riid,ppvObject) ) 

#define IAccessible2_AddRef(This)	\
    ( (This)->lpVtbl -> AddRef(This) ) 

#define IAccessible2_Release(This)	\
    ( (This)->lpVtbl -> Release(This) ) 


#define IAccessible2_GetTypeInfoCount(This,pctinfo)	\
    ( (This)->lpVtbl -> GetTypeInfoCount(This,pctinfo) ) 

#define IAccessible2_GetTypeInfo(This,iTInfo,lcid,ppTInfo)	\
    ( (This)->lpVtbl -> GetTypeInfo(This,iTInfo,lcid,ppTInfo) ) 

#define IAccessible2_GetIDsOfNames(This,riid,rgszNames,cNames,lcid,rgDispId)	\
    ( (This)->lpVtbl -> GetIDsOfNames(This,riid,rgszNames,cNames,lcid,rgDispId) ) 

#define IAccessible2_Invoke(This,dispIdMember,riid,lcid,wFlags,pDispParams,pVarResult,pExcepInfo,puArgErr)	\
    ( (This)->lpVtbl -> Invoke(This,dispIdMember,riid,lcid,wFlags,pDispParams,pVarResult,pExcepInfo,puArgErr) ) 


#define IAccessible2_get_accParent(This,ppdispParent)	\
    ( (This)->lpVtbl -> get_accParent(This,ppdispParent) ) 

#define IAccessible2_get_accChildCount(This,pcountChildren)	\
    ( (This)->lpVtbl -> get_accChildCount(This,pcountChildren) ) 

#define IAccessible2_get_accChild(This,varChild,ppdispChild)	\
    ( (This)->lpVtbl -> get_accChild(This,varChild,ppdispChild) ) 

#define IAccessible2_get_accName(This,varChild,pszName)	\
    ( (This)->lpVtbl -> get_accName(This,varChild,pszName) ) 

#define IAccessible2_get_accValue(This,varChild,pszValue)	\
    ( (This)->lpVtbl -> get_accValue(This,varChild,pszValue) ) 

#define IAccessible2_get_accDescription(This,varChild,pszDescription)	\
    ( (This)->lpVtbl -> get_accDescription(This,varChild,pszDescription) ) 

#define IAccessible2_get_accRole(This,varChild,pvarRole)	\
    ( (This)->lpVtbl -> get_accRole(This,varChild,pvarRole) ) 

#define IAccessible2_get_accState(This,varChild,pvarState)	\
    ( (This)->lpVtbl -> get_accState(This,varChild,pvarState) ) 

#define IAccessible2_get_accHelp(This,varChild,pszHelp)	\
    ( (This)->lpVtbl -> get_accHelp(This,varChild,pszHelp) ) 

#define IAccessible2_get_accHelpTopic(This,pszHelpFile,varChild,pidTopic)	\
    ( (This)->lpVtbl -> get_accHelpTopic(This,pszHelpFile,varChild,pidTopic) ) 

#define IAccessible2_get_accKeyboardShortcut(This,varChild,pszKeyboardShortcut)	\
    ( (This)->lpVtbl -> get_accKeyboardShortcut(This,varChild,pszKeyboardShortcut) ) 

#define IAccessible2_get_accFocus(This,pvarChild)	\
    ( (This)->lpVtbl -> get_accFocus(This,pvarChild) ) 

#define IAccessible2_get_accSelection(This,pvarChildren)	\
    ( (This)->lpVtbl -> get_accSelection(This,pvarChildren) ) 

#define IAccessible2_get_accDefaultAction(This,varChild,pszDefaultAction)	\
    ( (This)->lpVtbl -> get_accDefaultAction(This,varChild,pszDefaultAction) ) 

#define IAccessible2_accSelect(This,flagsSelect,varChild)	\
    ( (This)->lpVtbl -> accSelect(This,flagsSelect,varChild) ) 

#define IAccessible2_accLocation(This,pxLeft,pyTop,pcxWidth,pcyHeight,varChild)	\
    ( (This)->lpVtbl -> accLocation(This,pxLeft,pyTop,pcxWidth,pcyHeight,varChild) ) 

#define IAccessible2_accNavigate(This,navDir,varStart,pvarEndUpAt)	\
    ( (This)->lpVtbl -> accNavigate(This,navDir,varStart,pvarEndUpAt) ) 

#define IAccessible2_accHitTest(This,xLeft,yTop,pvarChild)	\
    ( (This)->lpVtbl -> accHitTest(This,xLeft,yTop,pvarChild) ) 

#define IAccessible2_accDoDefaultAction(This,varChild)	\
    ( (This)->lpVtbl -> accDoDefaultAction(This,varChild) ) 

#define IAccessible2_put_accName(This,varChild,szName)	\
    ( (This)->lpVtbl -> put_accName(This,varChild,szName) ) 

#define IAccessible2_put_accValue(This,varChild,szValue)	\
    ( (This)->lpVtbl -> put_accValue(This,varChild,szValue) ) 


#define IAccessible2_get_nRelations(This,nRelations)	\
    ( (This)->lpVtbl -> get_nRelations(This,nRelations) ) 

#define IAccessible2_get_relation(This,relationIndex,relation)	\
    ( (This)->lpVtbl -> get_relation(This,relationIndex,relation) ) 

#define IAccessible2_get_relations(This,maxRelations,relations,nRelations)	\
    ( (This)->lpVtbl -> get_relations(This,maxRelations,relations,nRelations) ) 

#define IAccessible2_role(This,role)	\
    ( (This)->lpVtbl -> role(This,role) ) 

#define IAccessible2_scrollTo(This,scrollType)	\
    ( (This)->lpVtbl -> scrollTo(This,scrollType) ) 

#define IAccessible2_scrollToPoint(This,coordinateType,x,y)	\
    ( (This)->lpVtbl -> scrollToPoint(This,coordinateType,x,y) ) 

#define IAccessible2_get_groupPosition(This,groupLevel,similarItemsInGroup,positionInGroup)	\
    ( (This)->lpVtbl -> get_groupPosition(This,groupLevel,similarItemsInGroup,positionInGroup) ) 

#define IAccessible2_get_states(This,states)	\
    ( (This)->lpVtbl -> get_states(This,states) ) 

#define IAccessible2_get_extendedRole(This,extendedRole)	\
    ( (This)->lpVtbl -> get_extendedRole(This,extendedRole) ) 

#define IAccessible2_get_localizedExtendedRole(This,localizedExtendedRole)	\
    ( (This)->lpVtbl -> get_localizedExtendedRole(This,localizedExtendedRole) ) 

#define IAccessible2_get_nExtendedStates(This,nExtendedStates)	\
    ( (This)->lpVtbl -> get_nExtendedStates(This,nExtendedStates) ) 

#define IAccessible2_get_extendedStates(This,maxExtendedStates,extendedStates,nExtendedStates)	\
    ( (This)->lpVtbl -> get_extendedStates(This,maxExtendedStates,extendedStates,nExtendedStates) ) 

#define IAccessible2_get_localizedExtendedStates(This,maxLocalizedExtendedStates,localizedExtendedStates,nLocalizedExtendedStates)	\
    ( (This)->lpVtbl -> get_localizedExtendedStates(This,maxLocalizedExtendedStates,localizedExtendedStates,nLocalizedExtendedStates) ) 

#define IAccessible2_get_uniqueID(This,uniqueID)	\
    ( (This)->lpVtbl -> get_uniqueID(This,uniqueID) ) 

#define IAccessible2_get_windowHandle(This,windowHandle)	\
    ( (This)->lpVtbl -> get_windowHandle(This,windowHandle) ) 

#define IAccessible2_get_indexInParent(This,indexInParent)	\
    ( (This)->lpVtbl -> get_indexInParent(This,indexInParent) ) 

#define IAccessible2_get_locale(This,locale)	\
    ( (This)->lpVtbl -> get_locale(This,locale) ) 

#define IAccessible2_get_attributes(This,attributes)	\
    ( (This)->lpVtbl -> get_attributes(This,attributes) ) 

#endif /* COBJMACROS */


#endif 	/* C style interface */




#endif 	/* __IAccessible2_INTERFACE_DEFINED__ */


/* User-marshal routines are declared by the imported Windows SDK headers. */

#ifdef __cplusplus
}
#endif

#endif


